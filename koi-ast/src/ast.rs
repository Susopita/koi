use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "nodeType")]
pub enum ASTNode {
    #[serde(rename = "program")]
    Program { children: Vec<ASTNode> },

    #[serde(rename = "function_def")]
    FunctionDef {
        name: String,
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "struct_def")]
    StructDef {
        name: String,
        fields: Vec<(String, String)>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "call")]
    Call {
        function: Box<ASTNode>,
        arguments: Vec<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "variable")]
    Variable {
        name: String,
        line: usize,
        column: usize,
    },

    #[serde(rename = "literal")]
    Literal {
        #[serde(rename = "literalType")]
        literal_type: String, // "int64", "float64", "bool", "string"
        value: serde_json::Value,
        line: usize,
        column: usize,
    },

    #[serde(rename = "lambda")]
    Lambda {
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "let_binding")]
    LetBinding {
        bindings: Vec<(String, Box<ASTNode>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "if")]
    IfExpr {
        condition: Box<ASTNode>,
        then_branch: Box<ASTNode>,
        else_branch: Option<Box<ASTNode>>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "loop")]
    LoopExpr {
        variable: String,
        init: Box<ASTNode>,
        condition: Box<ASTNode>,
        step: Box<ASTNode>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "field_access")]
    FieldAccess {
        object: Box<ASTNode>,
        field: String,
        line: usize,
        column: usize,
    },

    #[serde(rename = "index")]
    Index {
        array: Box<ASTNode>,
        index: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "addr_of")]
    AddrOf {
        operand: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "deref")]
    Deref {
        operand: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "new")]
    New {
        #[serde(rename = "typeStr")]
        type_str: String,
        #[serde(rename = "sizeOrInit")]
        size_or_init: Option<Box<ASTNode>>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "array_literal")]
    ArrayLiteral {
        elements: Vec<ASTNode>,
        line: usize,
        column: usize,
    },
}
