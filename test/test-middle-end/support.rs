//! Shared `ASTNode` construction helpers, included via `#[path]` by every
//! test file in this directory (each `[[test]]` target is compiled as its
//! own crate, so this can't just be a normal sibling module).
//!
//! koi-ir deliberately doesn't depend on koi-ast's parser (the two crates
//! only agree on the /tmp/ast.json shape), so tests build `ASTNode` values
//! directly rather than parsing `.carp` source.
#![allow(dead_code)]

use koi::frontend::ast::ASTNode;

pub fn program(children: Vec<ASTNode>) -> ASTNode {
    ASTNode::Program { children }
}

pub fn defn(name: &str, params: Vec<(&str, Option<&str>)>, body: ASTNode) -> ASTNode {
    ASTNode::FunctionDef {
        name: name.to_string(),
        parameters: params
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.map(str::to_string)))
            .collect(),
        body: Box::new(body),
        line: 1,
        column: 1,
    }
}

pub fn defstruct(name: &str, fields: Vec<(&str, &str)>) -> ASTNode {
    ASTNode::StructDef {
        name: name.to_string(),
        fields: fields
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
        line: 1,
        column: 1,
    }
}

pub fn var(name: &str) -> ASTNode {
    ASTNode::Variable {
        name: name.to_string(),
        line: 1,
        column: 1,
    }
}

pub fn int(n: i64) -> ASTNode {
    ASTNode::Literal {
        literal_type: "int64".to_string(),
        value: serde_json::json!(n),
        line: 1,
        column: 1,
    }
}

pub fn float(n: f64) -> ASTNode {
    ASTNode::Literal {
        literal_type: "float64".to_string(),
        value: serde_json::json!(n),
        line: 1,
        column: 1,
    }
}

pub fn bool_lit(b: bool) -> ASTNode {
    ASTNode::Literal {
        literal_type: "bool".to_string(),
        value: serde_json::json!(b),
        line: 1,
        column: 1,
    }
}

pub fn string_lit(s: &str) -> ASTNode {
    ASTNode::Literal {
        literal_type: "string".to_string(),
        value: serde_json::json!(s),
        line: 1,
        column: 1,
    }
}

pub fn call(function: ASTNode, arguments: Vec<ASTNode>) -> ASTNode {
    ASTNode::Call {
        function: Box::new(function),
        arguments,
        line: 1,
        column: 1,
    }
}

pub fn call_named(name: &str, arguments: Vec<ASTNode>) -> ASTNode {
    call(var(name), arguments)
}

pub fn if_expr(condition: ASTNode, then_branch: ASTNode, else_branch: Option<ASTNode>) -> ASTNode {
    ASTNode::IfExpr {
        condition: Box::new(condition),
        then_branch: Box::new(then_branch),
        else_branch: else_branch.map(Box::new),
        line: 1,
        column: 1,
    }
}

pub fn let_binding(bindings: Vec<(&str, ASTNode)>, body: ASTNode) -> ASTNode {
    ASTNode::LetBinding {
        bindings: bindings
            .into_iter()
            .map(|(n, v)| (n.to_string(), Box::new(v)))
            .collect(),
        body: Box::new(body),
        line: 1,
        column: 1,
    }
}

pub fn loop_expr(
    variable: &str,
    init: ASTNode,
    condition: ASTNode,
    step: ASTNode,
    body: ASTNode,
) -> ASTNode {
    ASTNode::LoopExpr {
        variable: variable.to_string(),
        init: Box::new(init),
        condition: Box::new(condition),
        step: Box::new(step),
        body: Box::new(body),
        line: 1,
        column: 1,
    }
}

pub fn lambda(params: Vec<(&str, Option<&str>)>, body: ASTNode) -> ASTNode {
    ASTNode::Lambda {
        parameters: params
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.map(str::to_string)))
            .collect(),
        body: Box::new(body),
        line: 1,
        column: 1,
    }
}

pub fn field_access(object: ASTNode, field: &str) -> ASTNode {
    ASTNode::FieldAccess {
        object: Box::new(object),
        field: field.to_string(),
        line: 1,
        column: 1,
    }
}

pub fn set_field(object: ASTNode, field: &str, value: ASTNode) -> ASTNode {
    ASTNode::SetField {
        object: Box::new(object),
        field: field.to_string(),
        value: Box::new(value),
        line: 1,
        column: 1,
    }
}

pub fn index(array: ASTNode, idx: ASTNode) -> ASTNode {
    ASTNode::Index {
        array: Box::new(array),
        index: Box::new(idx),
        line: 1,
        column: 1,
    }
}

pub fn addr_of(operand: ASTNode) -> ASTNode {
    ASTNode::AddrOf {
        operand: Box::new(operand),
        line: 1,
        column: 1,
    }
}

pub fn deref(operand: ASTNode) -> ASTNode {
    ASTNode::Deref {
        operand: Box::new(operand),
        line: 1,
        column: 1,
    }
}

pub fn new_expr(type_str: &str, size_or_init: Option<ASTNode>) -> ASTNode {
    ASTNode::New {
        type_str: type_str.to_string(),
        size_or_init: size_or_init.map(Box::new),
        line: 1,
        column: 1,
    }
}

pub fn array_literal(elements: Vec<ASTNode>) -> ASTNode {
    ASTNode::ArrayLiteral {
        elements,
        line: 1,
        column: 1,
    }
}

pub fn set_var(name: &str, value: ASTNode) -> ASTNode {
    ASTNode::SetVar {
        name: name.to_string(),
        value: Box::new(value),
        line: 1,
        column: 1,
    }
}

pub fn while_expr(condition: ASTNode, body: ASTNode) -> ASTNode {
    ASTNode::WhileExpr {
        condition: Box::new(condition),
        body: Box::new(body),
        line: 1,
        column: 1,
    }
}

pub fn do_expr(exprs: Vec<ASTNode>) -> ASTNode {
    ASTNode::DoExpr {
        exprs,
        line: 1,
        column: 1,
    }
}

/// Builds a `MakeClosure` node directly, exactly as `lambda_lifter.rs`
/// would produce it for a capturing lambda -- used by tests that exercise
/// `ir_generator.rs`'s closure-construction/closure-call handling without
/// going through the lifter itself.
pub fn make_closure(function_name: &str, captured: Vec<&str>) -> ASTNode {
    ASTNode::MakeClosure {
        function_name: function_name.to_string(),
        captured: captured.into_iter().map(str::to_string).collect(),
        line: 1,
        column: 1,
    }
}
