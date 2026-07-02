use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IRProgram {
    #[serde(rename = "irType")]
    pub ir_type: String,
    pub functions: Vec<IRFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IRFunction {
    pub name: String,
    #[serde(rename = "returnType")]
    pub return_type: String,
    pub parameters: Vec<(String, String)>,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicBlock {
    pub label: String,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Instruction {
    #[serde(rename = "const")]
    Const {
        result: String,
        value: Value,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "binop")]
    BinOp {
        result: String,
        lhs: String,
        rhs: String,
        #[serde(rename = "op_type")]
        op_type: String,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "call")]
    Call {
        result: Option<String>,
        function: String,
        arguments: Vec<String>,
        #[serde(rename = "type")]
        ty: Option<String>,
    },
    #[serde(rename = "return")]
    Return { value: Option<String> },
    #[serde(rename = "jump")]
    Jump { label: String },
    #[serde(rename = "branch")]
    Branch {
        cond: String,
        true_label: String,
        false_label: String,
    },
    #[serde(rename = "phi")]
    Phi {
        result: String,
        incoming: Vec<(String, String)>,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "call_indirect")]
    CallIndirect {
        result: Option<String>,
        function_value: String,
        arguments: Vec<String>,
        #[serde(rename = "type")]
        ty: Option<String>,
    },
    #[serde(rename = "alloc")]
    Alloc {
        result: String,
        #[serde(rename = "type")]
        ty: String,
        size: Option<String>,
    },
    #[serde(rename = "get_field")]
    GetField {
        result: String,
        object: String,
        field: String,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "get_index")]
    GetIndex {
        result: String,
        array: String,
        index: String,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "addr_of")]
    AddrOf {
        result: String,
        operand: String,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "deref")]
    Deref {
        result: String,
        operand: String,
        #[serde(rename = "type")]
        ty: String,
    },
}

impl Instruction {
    pub fn result_name(&self) -> Option<&str> {
        match self {
            Instruction::Const { result, .. }
            | Instruction::BinOp { result, .. }
            | Instruction::Phi { result, .. }
            | Instruction::Alloc { result, .. }
            | Instruction::GetField { result, .. }
            | Instruction::GetIndex { result, .. }
            | Instruction::AddrOf { result, .. }
            | Instruction::Deref { result, .. } => Some(result),
            Instruction::Call { result, .. } | Instruction::CallIndirect { result, .. } => {
                result.as_deref()
            }
            Instruction::Return { .. } | Instruction::Jump { .. } | Instruction::Branch { .. } => None,
        }
    }

    pub fn result_type(&self) -> Option<&str> {
        match self {
            Instruction::Const { ty, .. }
            | Instruction::BinOp { ty, .. }
            | Instruction::Phi { ty, .. }
            | Instruction::Alloc { ty, .. }
            | Instruction::GetField { ty, .. }
            | Instruction::GetIndex { ty, .. }
            | Instruction::AddrOf { ty, .. }
            | Instruction::Deref { ty, .. } => Some(ty),
            Instruction::Call { ty, .. } | Instruction::CallIndirect { ty, .. } => ty.as_deref(),
            Instruction::Return { .. } | Instruction::Jump { .. } | Instruction::Branch { .. } => None,
        }
    }
}

pub struct IRParser;

impl IRParser {
    pub fn parse_json(json_str: &str) -> Result<IRProgram, String> {
        let program: IRProgram =
            serde_json::from_str(json_str).map_err(|e| format!("invalid IR JSON: {e}"))?;
        if program.functions.is_empty() {
            return Err("IR program does not contain any functions".to_string());
        }
        Ok(program)
    }
}

