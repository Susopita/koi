use crate::frontend::ast::ASTNode;
use std::collections::{HashMap, HashSet};

/// Compiler intrinsics that are always "declared" -- they have no `defn` of
/// their own, koi-ir/koi-assembly special-case them.
const BUILTINS: &[&str] = &[
    "+", "-", "*", "/", "<", ">", "<=", ">=", "==", "!=", "&&", "||", "!", "print", "malloc",
    "free", "aset!",
];

pub struct ScopeAnalyzer {
    scopes: Vec<HashMap<String, String>>,
    functions: HashSet<String>,
    structs: HashSet<String>,
    errors: Vec<String>,
}

impl Default for ScopeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeAnalyzer {
    pub fn new() -> Self {
        ScopeAnalyzer {
            scopes: vec![HashMap::new()],
            functions: HashSet::new(),
            structs: HashSet::new(),
            errors: vec![],
        }
    }

    pub fn analyze(&mut self, node: &ASTNode) -> Result<(), Vec<String>> {
        // Pre-register every top-level defn/defstruct name first so mutual
        // recursion and forward references between functions work.
        self.collect_top_level(node);
        self.analyze_node(node);

        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }

    fn collect_top_level(&mut self, node: &ASTNode) {
        if let ASTNode::Program { children } = node {
            for child in children {
                match child {
                    ASTNode::FunctionDef { name, .. } => {
                        self.functions.insert(name.clone());
                    }
                    ASTNode::StructDef { name, .. } => {
                        self.structs.insert(name.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    fn is_builtin(name: &str) -> bool {
        BUILTINS.contains(&name)
    }

    fn declare(&mut self, name: &str) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), "local".to_string());
        }
    }

    fn is_declared(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|s| s.contains_key(name))
    }

    fn analyze_node(&mut self, node: &ASTNode) {
        match node {
            ASTNode::Program { children } => {
                for child in children {
                    self.analyze_node(child);
                }
            }
            ASTNode::FunctionDef {
                parameters, body, ..
            } => {
                self.scopes.push(HashMap::new());
                for (name, _) in parameters {
                    self.declare(name);
                }
                self.analyze_node(body);
                self.scopes.pop();
            }
            ASTNode::StructDef { .. } => {
                // Field names/types are not variables; nothing to check.
            }
            ASTNode::Call {
                function,
                arguments,
                ..
            } => {
                self.analyze_node(function);
                for arg in arguments {
                    self.analyze_node(arg);
                }
            }
            ASTNode::Variable { name, line, column } => {
                if !self.is_declared(name)
                    && !self.functions.contains(name)
                    && !Self::is_builtin(name)
                {
                    self.errors.push(format!(
                        "Variable '{}' not declared at line {}, column {}",
                        name, line, column
                    ));
                }
            }
            ASTNode::Literal { .. } => {}
            ASTNode::Lambda {
                parameters, body, ..
            } => {
                self.scopes.push(HashMap::new());
                for (name, _) in parameters {
                    self.declare(name);
                }
                self.analyze_node(body);
                self.scopes.pop();
            }
            ASTNode::LetBinding { bindings, body, .. } => {
                // Sequential (let*-style) scoping: each binding can see the
                // ones declared before it.
                self.scopes.push(HashMap::new());
                for (name, value) in bindings {
                    self.analyze_node(value);
                    self.declare(name);
                }
                self.analyze_node(body);
                self.scopes.pop();
            }
            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.analyze_node(condition);
                self.analyze_node(then_branch);
                if let Some(else_branch) = else_branch {
                    self.analyze_node(else_branch);
                }
            }
            ASTNode::LoopExpr {
                variable,
                init,
                condition,
                step,
                body,
                ..
            } => {
                self.analyze_node(init);
                self.scopes.push(HashMap::new());
                self.declare(variable);
                self.analyze_node(condition);
                self.analyze_node(step);
                self.analyze_node(body);
                self.scopes.pop();
            }
            ASTNode::FieldAccess { object, .. } => {
                self.analyze_node(object);
            }
            ASTNode::SetField { object, value, .. } => {
                self.analyze_node(object);
                self.analyze_node(value);
            }
            ASTNode::Index { array, index, .. } => {
                self.analyze_node(array);
                self.analyze_node(index);
            }
            ASTNode::AddrOf { operand, .. } | ASTNode::Deref { operand, .. } => {
                self.analyze_node(operand);
            }
            ASTNode::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    self.analyze_node(init);
                }
            }
            ASTNode::ArrayLiteral { elements, .. } => {
                for element in elements {
                    self.analyze_node(element);
                }
            }
            ASTNode::SetVar {
                name,
                value,
                line,
                column,
            } => {
                self.analyze_node(value);
                if !self.is_declared(name) && !self.functions.contains(name) && !Self::is_builtin(name)
                {
                    self.errors.push(format!(
                        "Variable '{}' not declared at line {}, column {}",
                        name, line, column
                    ));
                }
            }
            ASTNode::WhileExpr {
                condition, body, ..
            } => {
                self.analyze_node(condition);
                self.analyze_node(body);
            }
            ASTNode::DoExpr { exprs, .. } => {
                for e in exprs {
                    self.analyze_node(e);
                }
            }
            // MakeClosure is a compiler-internal node produced by the
            // lambda lifter, which runs long after scope analysis — it
            // should never appear here. The arm exists only for Rust's
            // exhaustiveness checker.
            ASTNode::MakeClosure { .. } => {}
        }
    }
}
