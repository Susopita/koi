use crate::frontend::ast::ASTNode;
use crate::middle_end::builtins::{BuiltinKind, builtin_kind};
use crate::middle_end::types::{Constraint, Type, TypeVar};
use std::collections::HashMap;

/// Type of a builtin referenced *as a value* rather than called directly
/// (e.g. never happens in the current test programs, but must not panic).
fn builtin_bare_type(kind: &BuiltinKind) -> Type {
    let fresh = || Type::Variable(TypeVar::fresh());
    match kind {
        BuiltinKind::Arith => Type::Function {
            params: vec![fresh(), fresh()],
            return_type: Box::new(fresh()),
        },
        BuiltinKind::Cmp => Type::Function {
            params: vec![fresh(), fresh()],
            return_type: Box::new(Type::Bool),
        },
        BuiltinKind::Logical => Type::Function {
            params: vec![Type::Bool, Type::Bool],
            return_type: Box::new(Type::Bool),
        },
        BuiltinKind::Not => Type::Function {
            params: vec![Type::Bool],
            return_type: Box::new(Type::Bool),
        },
        BuiltinKind::Print => Type::Function {
            params: vec![fresh()],
            return_type: Box::new(fresh()),
        },
        BuiltinKind::Malloc => Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Pointer(Box::new(fresh()))),
        },
        BuiltinKind::Free => Type::Function {
            params: vec![Type::Pointer(Box::new(fresh()))],
            return_type: Box::new(Type::Int64),
        },
        BuiltinKind::SetIndex => {
            let elem = fresh();
            Type::Function {
                params: vec![Type::Array(Box::new(elem.clone())), Type::Int64, elem],
                return_type: Box::new(Type::Unit),
            }
        }
    }
}

pub struct ConstraintGenerator {
    constraints: Vec<Constraint>,
    scopes: Vec<HashMap<String, Type>>,
    functions: HashMap<String, Type>,
    struct_fields: HashMap<String, Vec<(String, Type)>>,
}

impl Default for ConstraintGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintGenerator {
    pub fn new() -> Self {
        ConstraintGenerator {
            constraints: vec![],
            scopes: vec![],
            functions: HashMap::new(),
            struct_fields: HashMap::new(),
        }
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    pub fn functions(&self) -> &HashMap<String, Type> {
        &self.functions
    }

    pub fn struct_fields(&self) -> &HashMap<String, Vec<(String, Type)>> {
        &self.struct_fields
    }

    /// Resolves a raw type annotation string ("i64", "Point", ...) from the
    /// AST into a `Type`. Unknown names are optimistically treated as struct
    /// names (the user wrote a name on purpose; a fresh var would throw that
    /// information away) rather than erroring.
    fn parse_type_str(&self, s: &str) -> Type {
        match s {
            "i64" | "int64" => Type::Int64,
            "f64" | "float64" => Type::Float64,
            "bool" => Type::Bool,
            "string" => Type::String,
            other if other.starts_with("arr_") => {
                Type::Array(Box::new(self.parse_type_str(&other["arr_".len()..])))
            }
            other => Type::Struct(other.to_string()),
        }
    }

    fn declare(&mut self, name: &str, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        self.functions.get(name).cloned()
    }

    fn constrain(&mut self, lhs: Type, rhs: Type, context: &str, line: usize, column: usize) {
        self.constraints.push(Constraint {
            lhs,
            rhs,
            context: context.to_string(),
            line,
            column,
        });
    }

    pub fn generate_program(&mut self, program: &ASTNode) -> Result<(), String> {
        let children = match program {
            ASTNode::Program { children } => children,
            other => return Err(format!("expected top-level program, got {other:?}")),
        };

        // Pass 1: register struct field layouts (needed by `field`/`new`
        // resolution regardless of where the struct is declared).
        for child in children {
            if let ASTNode::StructDef { name, fields, .. } = child {
                let resolved = fields
                    .iter()
                    .map(|(fname, fty)| (fname.clone(), self.parse_type_str(fty)))
                    .collect();
                self.struct_fields.insert(name.clone(), resolved);
            }
        }

        // Pass 2: register every top-level function's signature *before*
        // walking any body, so forward references and recursion resolve.
        for child in children {
            if let ASTNode::FunctionDef {
                name, parameters, ..
            } = child
            {
                let param_types: Vec<Type> = parameters
                    .iter()
                    .map(|(_, ty)| match ty {
                        Some(s) => self.parse_type_str(s),
                        None => Type::Variable(TypeVar::fresh()),
                    })
                    .collect();
                let return_type = Type::Variable(TypeVar::fresh());
                self.functions.insert(
                    name.clone(),
                    Type::Function {
                        params: param_types,
                        return_type: Box::new(return_type),
                    },
                );
            }
        }

        // Pass 3: walk each function body in its own scope, constraining the
        // registered return type to what the body actually produces.
        for child in children {
            if let ASTNode::FunctionDef {
                name,
                parameters,
                body,
                line,
                column,
            } = child
            {
                let (param_types, return_type) = match self.functions.get(name) {
                    Some(Type::Function {
                        params,
                        return_type,
                    }) => (params.clone(), (**return_type).clone()),
                    _ => {
                        return Err(format!(
                            "internal error: missing signature for function '{name}'"
                        ));
                    }
                };

                self.scopes.push(HashMap::new());
                for ((pname, _), pty) in parameters.iter().zip(param_types.iter()) {
                    self.declare(pname, pty.clone());
                }
                let body_type = self.generate_expr(body)?;
                self.scopes.pop();

                self.constrain(
                    return_type,
                    body_type,
                    &format!("return type of '{name}'"),
                    *line,
                    *column,
                );
            }
        }

        Ok(())
    }

    fn generate_expr(&mut self, node: &ASTNode) -> Result<Type, String> {
        match node {
            ASTNode::Program { .. } | ASTNode::FunctionDef { .. } | ASTNode::StructDef { .. } => {
                Err(format!("'{:?}' cannot appear inside an expression", node))
            }

            // Mechanical-only arm, not a functional inference change:
            // `MakeClosure` is produced by `lambda_lifter.rs`, which runs
            // strictly *after* inference in the pipeline (see
            // `pipeline.rs`'s pass order) -- this stage never actually
            // receives one at runtime. This arm exists purely to satisfy
            // the compiler's exhaustiveness check now that `ASTNode` has
            // the new variant; it errors defensively, same as the
            // `Program`/`FunctionDef`/`StructDef` arm above, rather than
            // inventing a fake type for a node this stage should never see.
            ASTNode::MakeClosure { .. } => Err(format!(
                "'{node:?}' cannot appear before lambda-lifting"
            )),

            ASTNode::Literal { literal_type, .. } => Ok(match literal_type.as_str() {
                "int64" => Type::Int64,
                "float64" => Type::Float64,
                "bool" => Type::Bool,
                "string" => Type::String,
                _ => Type::Variable(TypeVar::fresh()),
            }),

            ASTNode::Variable { name, line, column } => {
                if let Some(ty) = self.lookup(name) {
                    return Ok(ty);
                }
                if let Some(kind) = builtin_kind(name) {
                    return Ok(builtin_bare_type(&kind));
                }
                Err(format!(
                    "Undefined variable: '{name}' at line {line}, column {column}"
                ))
            }

            ASTNode::Call {
                function,
                arguments,
                line,
                column,
            } => {
                if let ASTNode::Variable { name, .. } = function.as_ref()
                    && let Some(kind) = builtin_kind(name)
                {
                    return self.generate_builtin_call(&kind, arguments, *line, *column);
                }

                let func_type = self.generate_expr(function)?;
                let mut arg_types = vec![];
                for arg in arguments {
                    arg_types.push(self.generate_expr(arg)?);
                }

                let return_type = Type::Variable(TypeVar::fresh());
                self.constrain(
                    func_type,
                    Type::Function {
                        params: arg_types,
                        return_type: Box::new(return_type.clone()),
                    },
                    "function call",
                    *line,
                    *column,
                );
                Ok(return_type)
            }

            ASTNode::LetBinding { bindings, body, .. } => {
                self.scopes.push(HashMap::new());
                for (name, value) in bindings {
                    let ty = self.generate_expr(value)?;
                    self.declare(name, ty);
                }
                let result = self.generate_expr(body);
                self.scopes.pop();
                result
            }

            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                line,
                column,
            } => {
                let cond_type = self.generate_expr(condition)?;
                self.constrain(cond_type, Type::Bool, "if condition", *line, *column);

                let then_type = self.generate_expr(then_branch)?;

                if let Some(else_branch) = else_branch {
                    let else_type = self.generate_expr(else_branch)?;
                    self.constrain(then_type.clone(), else_type, "if branches", *line, *column);
                }

                Ok(then_type)
            }

            ASTNode::LoopExpr {
                variable,
                init,
                condition,
                step,
                body,
                line,
                column,
            } => {
                let init_type = self.generate_expr(init)?;

                self.scopes.push(HashMap::new());
                self.declare(variable, init_type.clone());

                let cond_type = self.generate_expr(condition)?;
                self.constrain(cond_type, Type::Bool, "loop condition", *line, *column);

                let step_type = self.generate_expr(step)?;
                self.constrain(
                    step_type,
                    init_type,
                    "loop step must match loop variable's type",
                    *line,
                    *column,
                );

                let body_type = self.generate_expr(body);
                self.scopes.pop();
                body_type
            }

            ASTNode::Lambda {
                parameters, body, ..
            } => {
                self.scopes.push(HashMap::new());
                let mut param_types = vec![];
                for (name, ty) in parameters {
                    let param_type = match ty {
                        Some(s) => self.parse_type_str(s),
                        None => Type::Variable(TypeVar::fresh()),
                    };
                    self.declare(name, param_type.clone());
                    param_types.push(param_type);
                }
                let body_type = self.generate_expr(body)?;
                self.scopes.pop();
                Ok(Type::Function {
                    params: param_types,
                    return_type: Box::new(body_type),
                })
            }

            ASTNode::FieldAccess {
                object,
                field,
                line,
                column,
            } => {
                let object_type = self.generate_expr(object)?;

                let matches: Vec<(&String, &Type)> = self
                    .struct_fields
                    .iter()
                    .filter_map(|(struct_name, fields)| {
                        fields
                            .iter()
                            .find(|(fname, _)| fname == field)
                            .map(|(_, fty)| (struct_name, fty))
                    })
                    .collect();

                if let [(struct_name, field_type)] = matches[..] {
                    let field_type = field_type.clone();
                    self.constrain(
                        object_type,
                        Type::Struct(struct_name.clone()),
                        &format!("field access '.{field}'"),
                        *line,
                        *column,
                    );
                    Ok(field_type)
                } else {
                    // Field name doesn't uniquely identify a struct (unknown
                    // or ambiguous) -- best-effort fresh type rather than an
                    // error, per "no panics on unknown shapes".
                    Ok(Type::Variable(TypeVar::fresh()))
                }
            }

            ASTNode::SetField {
                object,
                field,
                value,
                line,
                column,
            } => {
                let object_type = self.generate_expr(object)?;
                let value_type = self.generate_expr(value)?;

                let matches: Vec<(&String, &Type)> = self
                    .struct_fields
                    .iter()
                    .filter_map(|(struct_name, fields)| {
                        fields
                            .iter()
                            .find(|(fname, _)| fname == field)
                            .map(|(_, fty)| (struct_name, fty))
                    })
                    .collect();

                if let [(struct_name, field_type)] = matches[..] {
                    let field_type = field_type.clone();
                    self.constrain(
                        object_type,
                        Type::Struct(struct_name.clone()),
                        &format!("set-field! '.{field}'"),
                        *line,
                        *column,
                    );
                    self.constrain(
                        value_type,
                        field_type,
                        &format!("set-field! '.{field}' value type"),
                        *line,
                        *column,
                    );
                }
                Ok(Type::Unit)
            }

            ASTNode::Index {
                array,
                index,
                line,
                column,
            } => {
                let array_type = self.generate_expr(array)?;
                let elem_type = Type::Variable(TypeVar::fresh());
                self.constrain(
                    array_type,
                    Type::Array(Box::new(elem_type.clone())),
                    "indexed value must be an array",
                    *line,
                    *column,
                );

                let index_type = self.generate_expr(index)?;
                self.constrain(index_type, Type::Int64, "array index", *line, *column);

                Ok(elem_type)
            }

            ASTNode::AddrOf { operand, .. } => {
                let operand_type = self.generate_expr(operand)?;
                Ok(Type::Pointer(Box::new(operand_type)))
            }

            ASTNode::Deref {
                operand,
                line,
                column,
            } => {
                let operand_type = self.generate_expr(operand)?;
                let pointee_type = Type::Variable(TypeVar::fresh());
                self.constrain(
                    operand_type,
                    Type::Pointer(Box::new(pointee_type.clone())),
                    "dereferenced value must be a pointer",
                    *line,
                    *column,
                );
                Ok(pointee_type)
            }

            ASTNode::New {
                type_str,
                size_or_init,
                ..
            } => {
                if let Some(init) = size_or_init {
                    // Just type-check the size/init expression; the MVP
                    // grammar doesn't distinguish "size" from "initializer"
                    // syntactically, so there's no further constraint to add.
                    self.generate_expr(init)?;
                }

                let base_type = self.parse_type_str(type_str);
                Ok(match base_type {
                    Type::Struct(_) | Type::Array(_) => base_type,
                    other => Type::Pointer(Box::new(other)),
                })
            }

            ASTNode::ArrayLiteral {
                elements,
                line,
                column,
            } => {
                if elements.is_empty() {
                    return Ok(Type::Array(Box::new(Type::Variable(TypeVar::fresh()))));
                }

                let elem_type = self.generate_expr(&elements[0])?;
                for element in &elements[1..] {
                    let ty = self.generate_expr(element)?;
                    self.constrain(
                        elem_type.clone(),
                        ty,
                        "array elements must share a type",
                        *line,
                        *column,
                    );
                }

                Ok(Type::Array(Box::new(elem_type)))
            }

            ASTNode::SetVar {
                name,
                value,
                line,
                column,
            } => {
                let value_type = self.generate_expr(value)?;
                match self.lookup(name) {
                    Some(existing_type) => {
                        // Mutation must not change the binding's inferred
                        // type -- no annotation here, so the new value's
                        // type has to match whatever `name` was already
                        // declared as (same lookup mechanism `Variable` uses).
                        self.constrain(
                            existing_type,
                            value_type,
                            &format!("set! target '{name}' must keep its type"),
                            *line,
                            *column,
                        );
                        Ok(Type::Unit)
                    }
                    // koi-ast's scope analysis should already reject
                    // assignment to an undeclared name before this stage
                    // runs; handle it defensively rather than panicking.
                    None => Err(format!(
                        "set! target '{name}' is not declared at line {line}, column {column}"
                    )),
                }
            }

            ASTNode::WhileExpr {
                condition,
                body,
                line,
                column,
            } => {
                let cond_type = self.generate_expr(condition)?;
                self.constrain(cond_type, Type::Bool, "while condition", *line, *column);

                // Only the body's side effects/constraints matter -- its
                // value is discarded.
                self.generate_expr(body)?;

                Ok(Type::Unit)
            }

            ASTNode::DoExpr {
                exprs,
                line,
                column,
            } => {
                let Some((last, init)) = exprs.split_last() else {
                    return Err(format!(
                        "'do' requires at least one expression at line {line}, column {column}"
                    ));
                };

                for expr in init {
                    self.generate_expr(expr)?;
                }
                self.generate_expr(last)
            }
        }
    }

    fn generate_builtin_call(
        &mut self,
        kind: &BuiltinKind,
        arguments: &[ASTNode],
        line: usize,
        column: usize,
    ) -> Result<Type, String> {
        if matches!(kind, BuiltinKind::SetIndex) && arguments.len() != 3 {
            return Err(format!(
                "aset! expects exactly 3 arguments (array, index, value), got {} at line {line}, column {column}",
                arguments.len()
            ));
        }

        let mut arg_types = vec![];
        for arg in arguments {
            arg_types.push(self.generate_expr(arg)?);
        }

        match kind {
            BuiltinKind::Arith => match arg_types.first().cloned() {
                Some(first) => {
                    for ty in &arg_types[1..] {
                        self.constrain(
                            first.clone(),
                            ty.clone(),
                            "arithmetic operands",
                            line,
                            column,
                        );
                    }
                    Ok(first)
                }
                None => Ok(Type::Variable(TypeVar::fresh())),
            },
            BuiltinKind::Cmp => {
                if let Some(first) = arg_types.first().cloned() {
                    for ty in &arg_types[1..] {
                        self.constrain(
                            first.clone(),
                            ty.clone(),
                            "comparison operands",
                            line,
                            column,
                        );
                    }
                }
                Ok(Type::Bool)
            }
            BuiltinKind::Logical | BuiltinKind::Not => {
                for ty in &arg_types {
                    self.constrain(ty.clone(), Type::Bool, "logical operand", line, column);
                }
                Ok(Type::Bool)
            }
            BuiltinKind::Print => Ok(arg_types.into_iter().next().unwrap_or(Type::Int64)),
            BuiltinKind::Malloc => {
                if let Some(ty) = arg_types.first().cloned() {
                    self.constrain(ty, Type::Int64, "malloc size", line, column);
                }
                Ok(Type::Pointer(Box::new(Type::Variable(TypeVar::fresh()))))
            }
            BuiltinKind::Free => {
                if let Some(ty) = arg_types.first().cloned() {
                    let elem = Type::Variable(TypeVar::fresh());
                    self.constrain(
                        ty,
                        Type::Pointer(Box::new(elem)),
                        "free operand",
                        line,
                        column,
                    );
                }
                Ok(Type::Int64)
            }
            BuiltinKind::SetIndex => {
                let elem_ty = Type::Variable(TypeVar::fresh());

                let array_ty = arg_types[0].clone();
                self.constrain(
                    array_ty,
                    Type::Array(Box::new(elem_ty.clone())),
                    "aset! target must be an array",
                    line,
                    column,
                );

                let index_ty = arg_types[1].clone();
                self.constrain(index_ty, Type::Int64, "aset! index", line, column);

                let value_ty = arg_types[2].clone();
                self.constrain(value_ty, elem_ty, "aset! value must match element type", line, column);

                Ok(Type::Unit)
            }
        }
    }
}
