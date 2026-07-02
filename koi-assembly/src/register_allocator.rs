use crate::abi::AMD64ABI;
use crate::ir_parser::{IRFunction, Instruction};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueLocation {
    Stack(i64),
}

impl ValueLocation {
    pub fn as_operand(&self) -> String {
        match self {
            ValueLocation::Stack(offset) => format!("{offset}(%rbp)"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LiveInterval {
    pub var: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone)]
pub struct FunctionLayout {
    pub locations: HashMap<String, ValueLocation>,
    pub value_types: HashMap<String, String>,
    pub stack_size: i64,
}

pub struct LinearScanAllocator;

impl LinearScanAllocator {
    pub fn new() -> Self {
        LinearScanAllocator
    }

    pub fn allocate(&self, function: &IRFunction) -> FunctionLayout {
        let intervals = self.compute_live_intervals(function);
        let mut value_types = HashMap::new();

        for (name, ty) in &function.parameters {
            value_types.insert(name.clone(), ty.clone());
        }
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let (Some(result), Some(ty)) = (instruction.result_name(), instruction.result_type()) {
                    value_types.insert(result.to_string(), ty.to_string());
                }
            }
        }

        let mut locations = HashMap::new();
        let mut next_slot = -8i64;

        for (name, _) in &function.parameters {
            locations.insert(name.clone(), ValueLocation::Stack(next_slot));
            next_slot -= 8;
        }

        for interval in intervals {
            if locations.contains_key(&interval.var) {
                continue;
            }
            if !value_types.contains_key(&interval.var) {
                continue;
            }
            locations.insert(interval.var, ValueLocation::Stack(next_slot));
            next_slot -= 8;
        }

        let used_bytes = (-next_slot - 8).max(0);
        FunctionLayout {
            locations,
            value_types,
            stack_size: AMD64ABI::align_to_16(used_bytes),
        }
    }

    pub fn compute_live_intervals(&self, function: &IRFunction) -> Vec<LiveInterval> {
        let mut starts = HashMap::<String, usize>::new();
        let mut ends = HashMap::<String, usize>::new();
        let mut position = 0usize;

        for (param_name, _) in &function.parameters {
            starts.entry(param_name.clone()).or_insert(position);
            ends.insert(param_name.clone(), position);
            position += 1;
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
            })
            .collect();
        intervals.sort_by_key(|interval| (interval.start, interval.end));
        intervals
    }
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
