//! Mirrors koi-ast's `ASTNode` exactly. koi-ir does not link against koi-ast
//! (the two crates are decoupled and only agree on the /tmp/ast.json shape),
//! so this is a deliberate duplication of the schema, not a shared type.

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
        literal_type: String,
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

    #[serde(rename = "set_field")]
    SetField {
        object: Box<ASTNode>,
        field: String,
        value: Box<ASTNode>,
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

    #[serde(rename = "set")]
    SetVar {
        name: String,
        value: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "while")]
    WhileExpr {
        condition: Box<ASTNode>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },

    #[serde(rename = "do")]
    DoExpr {
        exprs: Vec<ASTNode>,
        line: usize,
        column: usize,
    },

    /// Compiler-internal only: koi-ast has no concept of closures, so this
    /// variant never round-trips through real `/tmp/ast.json` from koi-ast.
    /// It's produced by `lambda_lifter.rs` in place of a capturing `Lambda`
    /// (replacing the old placeholder `Call` to a made-up
    /// `__make_closure_*` function name) and consumed only by
    /// `ir_generator.rs`, which runs strictly after monomorphization and
    /// lambda-lifting -- by that point every captured variable's type is
    /// already concrete, so the actual closure construction (env struct
    /// alloc + field stores + the shared `Closure` wrapper) can happen
    /// there with real types in hand instead of the lifter's placeholder
    /// i64-for-everything guess. Still tagged/derived like every other
    /// variant for consistency, even though the tag is never exercised by
    /// external JSON.
    #[serde(rename = "make_closure")]
    MakeClosure {
        function_name: String,
        captured: Vec<String>,
        line: usize,
        column: usize,
    },
}
