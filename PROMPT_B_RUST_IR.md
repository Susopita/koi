# **🦀 PROMPT B: RUST TYPE SYSTEM & IR — HM, Monomorphization, Lambda Lifting**

**Responsabilidad:** Implementar type inference Hindley-Milner, monomorphization, lambda lifting, e IR generation (HIR/LIR) en **Rust**.

**Entrada:** `/tmp/ast.json` (desde Persona A)

**Salida:** `/tmp/ir.json` (IR tipado)

**Timeline:** Días 2-6 de 1 semana

**Crate:** `koi-ir` (binario)

---

## **Estructura del Crate**

```
koi-ir/
├── Cargo.toml
└── src/
    ├── main.rs           (entry point, lee AST JSON)
    ├── types.rs          (Type, TypeVar, Type environments)
    ├── inference.rs      (Constraint generation)
    ├── unification.rs    (Robinson unification)
    ├── monomorphizer.rs  (Specialization)
    ├── lambda_lifter.rs  (Closure conversion)
    ├── ir.rs             (HIR/LIR structures)
    └── ir_generator.rs   (AST → IR transformation)
```

---

## **Parte 1: Type System en Rust**

### **Tipos Base (types.rs)**

```rust
// koi-ir/src/types.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Type {
    Int64,
    Float64,
    Bool,
    String,
    Array(Box<Type>),
    Pointer(Box<Type>),
    Struct(String),
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
    Variable(TypeVar),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypeVar {
    id: usize,
}

static mut TYPE_VAR_COUNTER: usize = 0;

impl TypeVar {
    pub fn fresh() -> Self {
        unsafe {
            let id = TYPE_VAR_COUNTER;
            TYPE_VAR_COUNTER += 1;
            TypeVar { id }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Constraint {
    pub lhs: Type,
    pub rhs: Type,
    pub context: String,
}

#[derive(Debug, Clone)]
pub struct Substitution {
    bindings: HashMap<TypeVar, Type>,
}

impl Substitution {
    pub fn new() -> Self {
        Substitution {
            bindings: HashMap::new(),
        }
    }
    
    pub fn apply(&self, ty: &Type) -> Type {
        match ty {
            Type::Variable(var) => {
                if let Some(bound) = self.bindings.get(var) {
                    self.apply(bound)
                } else {
                    ty.clone()
                }
            }
            Type::Array(elem) => Type::Array(Box::new(self.apply(elem))),
            Type::Pointer(elem) => Type::Pointer(Box::new(self.apply(elem))),
            Type::Function { params, return_type } => {
                Type::Function {
                    params: params.iter().map(|p| self.apply(p)).collect(),
                    return_type: Box::new(self.apply(return_type)),
                }
            }
            _ => ty.clone(),
        }
    }
    
    pub fn compose(&self, other: &Substitution) -> Substitution {
        let mut new_bindings = other.bindings.clone();
        
        for (var, ty) in &self.bindings {
            new_bindings.insert(*var, other.apply(ty));
        }
        
        Substitution {
            bindings: new_bindings,
        }
    }
    
    pub fn bind(&mut self, var: TypeVar, ty: Type) -> Result<(), String> {
        if self.occurs_check(&var, &ty) {
            Err(format!("Infinite type: ?T{} = {:?}", var.id, ty))
        } else {
            self.bindings.insert(var, ty);
            Ok(())
        }
    }
    
    fn occurs_check(&self, var: &TypeVar, ty: &Type) -> bool {
        match ty {
            Type::Variable(v) => v == var,
            Type::Array(elem) => self.occurs_check(var, elem),
            Type::Pointer(elem) => self.occurs_check(var, elem),
            Type::Function { params, return_type } => {
                params.iter().any(|p| self.occurs_check(var, p))
                    || self.occurs_check(var, return_type)
            }
            _ => false,
        }
    }
}
```

---

## **Parte 2: Hindley-Milner Inference**

### **Constraint Generation (inference.rs)**

```rust
// koi-ir/src/inference.rs

use crate::ast::ASTNode;
use crate::types::{Type, TypeVar, Constraint, Substitution};
use std::collections::HashMap;

pub struct ConstraintGenerator {
    constraints: Vec<Constraint>,
    environment: HashMap<String, Type>,
}

impl ConstraintGenerator {
    pub fn new() -> Self {
        ConstraintGenerator {
            constraints: vec![],
            environment: HashMap::new(),
        }
    }
    
    pub fn generate(&mut self, node: &ASTNode) -> Result<Type, String> {
        self.generate_expr(node)
    }
    
    fn generate_expr(&mut self, node: &ASTNode) -> Result<Type, String> {
        match node {
            ASTNode::Literal { literal_type, .. } => {
                Ok(match literal_type.as_str() {
                    "int64" => Type::Int64,
                    "float64" => Type::Float64,
                    "bool" => Type::Bool,
                    "string" => Type::String,
                    _ => Type::Variable(TypeVar::fresh()),
                })
            }
            ASTNode::Variable { name, .. } => {
                if let Some(ty) = self.environment.get(name) {
                    Ok(ty.clone())
                } else {
                    Err(format!("Undefined variable: {}", name))
                }
            }
            ASTNode::Call { function, arguments, .. } => {
                let func_type = self.generate_expr(function)?;
                let mut arg_types = vec![];
                
                for arg in arguments {
                    arg_types.push(self.generate_expr(arg)?);
                }
                
                let return_type = Type::Variable(TypeVar::fresh());
                
                // Constraint: func_type = arg_types -> return_type
                self.constraints.push(Constraint {
                    lhs: func_type,
                    rhs: Type::Function {
                        params: arg_types,
                        return_type: Box::new(return_type.clone()),
                    },
                    context: "function call".to_string(),
                });
                
                Ok(return_type)
            }
            ASTNode::LetBinding { bindings, body, .. } => {
                for (var_name, value) in bindings {
                    let ty = self.generate_expr(value)?;
                    self.environment.insert(var_name.clone(), ty);
                }
                
                self.generate_expr(body)
            }
            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let cond_type = self.generate_expr(condition)?;
                
                // Condition must be bool
                self.constraints.push(Constraint {
                    lhs: cond_type,
                    rhs: Type::Bool,
                    context: "if condition".to_string(),
                });
                
                let then_type = self.generate_expr(then_branch)?;
                
                let result_type = if let Some(els) = else_branch {
                    let else_type = self.generate_expr(els)?;
                    
                    // Both branches must have same type
                    self.constraints.push(Constraint {
                        lhs: then_type.clone(),
                        rhs: else_type,
                        context: "if branches".to_string(),
                    });
                    
                    then_type
                } else {
                    then_type
                };
                
                Ok(result_type)
            }
            // ... more cases (lambda, struct, etc.)
            _ => Ok(Type::Variable(TypeVar::fresh())),
        }
    }
    
    pub fn get_constraints(&self) -> Vec<Constraint> {
        self.constraints.clone()
    }
}
```

### **Unification (unification.rs)**

```rust
// koi-ir/src/unification.rs

use crate::types::{Type, TypeVar, Constraint, Substitution};

pub struct Unifier;

impl Unifier {
    pub fn unify(constraints: Vec<Constraint>) -> Result<Substitution, String> {
        let mut subst = Substitution::new();
        
        for constraint in constraints {
            let lhs = subst.apply(&constraint.lhs);
            let rhs = subst.apply(&constraint.rhs);
            
            Self::unify_types(&mut subst, &lhs, &rhs)?;
        }
        
        Ok(subst)
    }
    
    fn unify_types(
        subst: &mut Substitution,
        lhs: &Type,
        rhs: &Type,
    ) -> Result<(), String> {
        match (lhs, rhs) {
            // Same concrete types
            (Type::Int64, Type::Int64) => Ok(()),
            (Type::Float64, Type::Float64) => Ok(()),
            (Type::Bool, Type::Bool) => Ok(()),
            (Type::String, Type::String) => Ok(()),
            
            // Type variable unification
            (Type::Variable(v), t) | (t, Type::Variable(v)) => {
                if let Type::Variable(v2) = t {
                    if v == v2 {
                        return Ok(());
                    }
                }
                subst.bind(*v, t.clone())
            }
            
            // Compound types
            (Type::Array(a), Type::Array(b)) => {
                Self::unify_types(subst, a, b)
            }
            (Type::Pointer(a), Type::Pointer(b)) => {
                Self::unify_types(subst, a, b)
            }
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                },
            ) => {
                if p1.len() != p2.len() {
                    return Err(format!(
                        "Function arity mismatch: {} vs {}",
                        p1.len(),
                        p2.len()
                    ));
                }
                
                for (a, b) in p1.iter().zip(p2.iter()) {
                    Self::unify_types(subst, a, b)?;
                }
                
                Self::unify_types(subst, r1, r2)
            }
            
            _ => Err(format!(
                "Type mismatch: {:?} vs {:?}",
                lhs, rhs
            )),
        }
    }
}
```

---

## **Parte 3: Monomorphization (monomorphizer.rs)**

```rust
// koi-ir/src/monomorphizer.rs

use crate::ast::ASTNode;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

pub struct Monomorphizer {
    specializations: HashMap<String, Vec<Type>>,
}

impl Monomorphizer {
    pub fn new() -> Self {
        Monomorphizer {
            specializations: HashMap::new(),
        }
    }
    
    pub fn monomorphize(&mut self, node: &ASTNode) -> ASTNode {
        // First pass: detect all function calls and their type arguments
        self.detect_specializations(node);
        
        // Second pass: generate specialized functions
        self.generate_specializations(node)
    }
    
    fn detect_specializations(&mut self, node: &ASTNode) {
        match node {
            ASTNode::Call { function, arguments, .. } => {
                if let ASTNode::Variable { name, .. } = &**function {
                    // Record this specialization
                    let arg_types: Vec<Type> = arguments
                        .iter()
                        .filter_map(|arg| self.extract_type(arg))
                        .collect();
                    
                    self.specializations
                        .entry(name.clone())
                        .or_insert_with(Vec::new)
                        .extend(arg_types);
                }
                
                self.detect_specializations(function);
                for arg in arguments {
                    self.detect_specializations(arg);
                }
            }
            // Recursively walk tree...
            _ => {}
        }
    }
    
    fn generate_specializations(&mut self, node: &ASTNode) -> ASTNode {
        // Clone functions with specialized type parameters
        // This is where name mangling happens
        node.clone() // Simplified; real implementation would clone + specialize
    }
    
    fn extract_type(&self, _node: &ASTNode) -> Option<Type> {
        // Extract inferred type from node
        None // Placeholder
    }
    
    fn mangle_name(base: &str, type_args: &[Type]) -> String {
        let type_strings: Vec<String> = type_args
            .iter()
            .map(|t| format!("{:?}", t))
            .collect();
        format!("{}___{}", base, type_strings.join("__"))
    }
}
```

---

## **Parte 4: Lambda Lifting (lambda_lifter.rs)**

```rust
// koi-ir/src/lambda_lifter.rs

use crate::ast::ASTNode;
use std::collections::{HashMap, HashSet};

pub struct LambdaLifter {
    lifted_functions: Vec<ASTNode>,
    lambda_counter: usize,
}

impl LambdaLifter {
    pub fn new() -> Self {
        LambdaLifter {
            lifted_functions: vec![],
            lambda_counter: 0,
        }
    }
    
    pub fn lift(&mut self, node: &ASTNode) -> ASTNode {
        self.lift_node(node)
    }
    
    fn lift_node(&mut self, node: &ASTNode) -> ASTNode {
        match node {
            ASTNode::Lambda { parameters, body, line, column } => {
                // Find free variables (variables used in body but not in parameters)
                let free_vars = self.find_free_variables(body, parameters);
                
                // Create lifted function
                let func_name = format!("_lambda_{}", self.lambda_counter);
                self.lambda_counter += 1;
                
                // Create environment struct
                let env_struct_name = format!("_Lambda_{}_Env", self.lambda_counter - 1);
                
                // Create lifted function with env as first parameter
                let mut lifted_params = vec![
                    ("env".to_string(), Some(env_struct_name.clone())),
                ];
                lifted_params.extend(parameters.clone());
                
                // Rewrite body to access free vars through env
                let rewritten_body = self.rewrite_free_var_access(body, &free_vars);
                
                let lifted_func = ASTNode::FunctionDef {
                    name: func_name.clone(),
                    parameters: lifted_params,
                    body: Box::new(rewritten_body),
                    line: *line,
                    column: *column,
                };
                
                self.lifted_functions.push(lifted_func);
                
                // Replace lambda with fat pointer creation
                ASTNode::New {
                    type_str: "fat_ptr".to_string(),
                    size_or_init: None,
                    line: *line,
                    column: *column,
                }
            }
            // Recursively lift in other nodes...
            ASTNode::Call { function, arguments, line, column } => {
                ASTNode::Call {
                    function: Box::new(self.lift_node(function)),
                    arguments: arguments.iter().map(|a| self.lift_node(a)).collect(),
                    line: *line,
                    column: *column,
                }
            }
            // ... more cases
            _ => node.clone(),
        }
    }
    
    fn find_free_variables(
        &self,
        body: &ASTNode,
        params: &[(String, Option<String>)],
    ) -> HashSet<String> {
        let param_names: HashSet<String> = params.iter().map(|(n, _)| n.clone()).collect();
        self.find_vars_in_node(body, &param_names)
    }
    
    fn find_vars_in_node(
        &self,
        node: &ASTNode,
        bound_vars: &HashSet<String>,
    ) -> HashSet<String> {
        let mut free_vars = HashSet::new();
        
        match node {
            ASTNode::Variable { name, .. } => {
                if !bound_vars.contains(name) {
                    free_vars.insert(name.clone());
                }
            }
            ASTNode::Call { function, arguments, .. } => {
                free_vars.extend(self.find_vars_in_node(function, bound_vars));
                for arg in arguments {
                    free_vars.extend(self.find_vars_in_node(arg, bound_vars));
                }
            }
            // ... recursively analyze other node types
            _ => {}
        }
        
        free_vars
    }
    
    fn rewrite_free_var_access(
        &self,
        node: &ASTNode,
        free_vars: &HashSet<String>,
    ) -> ASTNode {
        match node {
            ASTNode::Variable { name, line, column } => {
                if free_vars.contains(name) {
                    // Rewrite as (. env name)
                    ASTNode::FieldAccess {
                        object: Box::new(ASTNode::Variable {
                            name: "env".to_string(),
                            line: *line,
                            column: *column,
                        }),
                        field: name.clone(),
                        line: *line,
                        column: *column,
                    }
                } else {
                    node.clone()
                }
            }
            // Recursively rewrite in compound nodes...
            _ => node.clone(),
        }
    }
    
    pub fn get_lifted_functions(&self) -> Vec<ASTNode> {
        self.lifted_functions.clone()
    }
}
```

---

## **Parte 5: IR Structures (ir.rs)**

```rust
// koi-ir/src/ir.rs

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IRProgram {
    #[serde(rename = "irType")]
    pub ir_type: String, // "hir" o "lir"
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
}
```

---

## **Parte 6: IR Generation (ir_generator.rs)**

```rust
// koi-ir/src/ir_generator.rs

use crate::ast::ASTNode;
use crate::ir::*;
use std::collections::HashMap;

pub struct IRGenerator {
    temp_counter: usize,
    label_counter: usize,
    current_block: Vec<Instruction>,
}

impl IRGenerator {
    pub fn new() -> Self {
        IRGenerator {
            temp_counter: 0,
            label_counter: 0,
            current_block: vec![],
        }
    }
    
    pub fn generate(&mut self, node: &ASTNode) -> Result<IRProgram, String> {
        let mut functions = vec![];
        
        if let ASTNode::Program { children } = node {
            for child in children {
                if let ASTNode::FunctionDef {
                    name,
                    parameters,
                    body,
                    ..
                } = child
                {
                    let func = self.generate_function(
                        name.clone(),
                        parameters.clone(),
                        body.clone(),
                    )?;
                    functions.push(func);
                }
            }
        }
        
        Ok(IRProgram {
            ir_type: "hir".to_string(),
            functions,
        })
    }
    
    fn generate_function(
        &mut self,
        name: String,
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
    ) -> Result<IRFunction, String> {
        self.current_block.clear();
        
        self.generate_expr(&body)?;
        
        Ok(IRFunction {
            name,
            return_type: "i64".to_string(),
            parameters: parameters
                .into_iter()
                .map(|(n, t)| (n, t.unwrap_or_else(|| "i64".to_string())))
                .collect(),
            blocks: vec![BasicBlock {
                label: "entry".to_string(),
                instructions: self.current_block.clone(),
            }],
        })
    }
    
    fn generate_expr(&mut self, node: &ASTNode) -> Result<String, String> {
        match node {
            ASTNode::Literal { literal_type, value, .. } => {
                let result = self.new_temp();
                self.current_block.push(Instruction::Const {
                    result: result.clone(),
                    value: value.clone(),
                    ty: literal_type.clone(),
                });
                Ok(result)
            }
            ASTNode::Call { function, arguments, .. } => {
                if let ASTNode::Variable { name, .. } = &**function {
                    let mut arg_temps = vec![];
                    for arg in arguments {
                        arg_temps.push(self.generate_expr(arg)?);
                    }
                    
                    let result = self.new_temp();
                    self.current_block.push(Instruction::Call {
                        result: Some(result.clone()),
                        function: name.clone(),
                        arguments: arg_temps,
                        ty: Some("i64".to_string()),
                    });
                    Ok(result)
                } else {
                    Err("Complex function calls not yet supported".to_string())
                }
            }
            // ... more cases
            _ => Ok(self.new_temp()),
        }
    }
    
    fn new_temp(&mut self) -> String {
        let temp = format!("%v{}", self.temp_counter);
        self.temp_counter += 1;
        temp
    }
    
    fn new_label(&mut self) -> String {
        let label = format!("label_{}", self.label_counter);
        self.label_counter += 1;
        label
    }
}
```

---

## **Main Entry Point (main.rs)**

```rust
// koi-ir/src/main.rs

mod types;
mod inference;
mod unification;
mod monomorphizer;
mod lambda_lifter;
mod ir;
mod ir_generator;

use std::fs;
use crate::inference::ConstraintGenerator;
use crate::unification::Unifier;
use crate::ir_generator::IRGenerator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read AST JSON from Persona A
    let ast_json = fs::read_to_string("/tmp/ast.json")?;
    let ast: serde_json::Value = serde_json::from_str(&ast_json)?;
    
    println!("✓ IR starting...");
    
    // Type inference
    let mut constraint_gen = ConstraintGenerator::new();
    // Note: would need to convert JSON back to ASTNode
    // For MVP, this is simplified
    
    // Unification
    let _subst = Unifier::unify(constraint_gen.get_constraints())?;
    
    // IR generation
    let mut ir_gen = IRGenerator::new();
    // Would process AST here
    
    println!("✓ IR complete.");
    
    // Write IR JSON
    // fs::write("/tmp/ir.json", ir_json)?;
    
    Ok(())
}
```

---

## **Cargo.toml**

```toml
[package]
name = "koi-ir"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

---

## **Checklist Koi-IR en Rust (5 días)**

- [ ] Día 2: Type enum + Substitution
- [ ] Día 3: Constraint generation + Unification
- [ ] Día 4: Monomorphization
- [ ] Día 5: Lambda lifting + IR generation
- [ ] Día 6: /tmp/ir.json válido

¡Listos para construir Koi-IR en Rust! 🦀
