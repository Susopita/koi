use crate::ir_parser::{IRProgram, IRFunction, Instruction};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub struct Optimizer;

impl Optimizer {
    pub fn optimize_program(program: &mut IRProgram) {
        for function in &mut program.functions {
            Self::optimize_function(function);
        }
    }

    pub fn optimize_function(function: &mut IRFunction) {
        Self::constant_folding(function);
        Self::dead_code_elimination(function);
    }

    fn constant_folding(function: &mut IRFunction) {
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
    }

    fn dead_code_elimination(function: &mut IRFunction) {
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

        for block in function.blocks.iter_mut().rev() {
            let mut kept = Vec::with_capacity(block.instructions.len());
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
            block.instructions = kept;
        }
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
        Instruction::GetIndex { array, index, .. } => vec![array.clone(), index.clone()],
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
