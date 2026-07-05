//! Hindley-Milner type inference over [`TypedExpr`].
//!
//! Each [`TypedExpr`] node starts with a fresh type variable.  The inferer
//! walks the tree, generates equality constraints (`lhs = rhs`), unifies
//! them via [`Unifier`], then walks again to replace every type variable
//! with its resolved concrete type.
//!
//! # Constraints generated per node
//!
//! | Node | Constraints |
//! |---|---|
//! | `Int` | `node_type = Int64` |
//! | `Float` | `node_type = Float64` |
//! | `Bool` / `Str` | `node_type = Bool` / `String` |
//! | `Var` | `node_type = lookup(name)` |
//! | `Let` | infer binding values, extend scope, node_type = body_type |
//! | `Set` | node_type = Unit, value_type = lookup(name) |
//! | `Lambda` | node_type = Function(params, body_type) |
//! | `App` | `function_type = Function(arg_types, node_type)` |
//! | `If` | `cond_type = Bool`, `then_type = else_type = node_type` |
//! | `While` | `cond_type = Bool`, node_type = Unit |
//! | `Loop` | `var_type = init_type = step_type`, `cond_type = Bool`, `node_type = fresh` |
//! | `Do` | node_type = type of last expression |
//! | `Array` | element types unified, node_type = Array(elem_type) |
//! | `New` | node_type = Pointer(allocated_type) |
//! | `Field` | node_type = field_type from struct definition |
//! | `SetField` | value_type = field_type, node_type = Unit |
//! | `Index` | `array_type = Array(node_type)` |
//! | `AddrOf` | node_type = Pointer(operand_type) |
//! | `Deref` | `operand_type = Pointer(node_type)` |

use std::collections::HashMap;

use crate::frontend::typed_ast::{TopLevel, TypedExpr};
use crate::middle_end::builtins::{builtin_kind, BuiltinKind};
use crate::middle_end::types::{Constraint, Type, TypeVar};
use crate::middle_end::unification::Unifier;

/// The type-inference engine.
///
/// Collects constraints while walking the [`TypedExpr`] tree, then unifies
/// them in a single batch and resolves every node's type variable.
pub struct TypeInferer {
    constraints: Vec<Constraint>,
    scopes: Vec<HashMap<String, Type>>,
    struct_fields: HashMap<String, Vec<(String, Type)>>,
}

impl TypeInferer {
    pub fn new() -> Self {
        TypeInferer {
            constraints: Vec::new(),
            scopes: Vec::new(),
            struct_fields: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Program-level entry point
    // ------------------------------------------------------------------

    /// Run full type inference on a program, resolving all type variables
    /// in every [`TypedExpr`] node in-place.
    pub fn infer_program(&mut self, toplevels: &mut [TopLevel]) -> Result<(), String> {
        // Pass 1: collect struct definitions.
        for tl in toplevels.iter() {
            if let TopLevel::Struct { name, fields } = tl {
                self.struct_fields.insert(name.clone(), fields.clone());
            }
        }

        // Pass 2: register function signatures (for forward references).
        let sigs: Vec<(String, Vec<Type>)> = toplevels
            .iter()
            .filter_map(|tl| match tl {
                TopLevel::Defn { name, parameters, .. } => {
                    let param_types: Vec<Type> = parameters
                        .iter()
                        .map(|(_, ty)| {
                            ty.clone().unwrap_or_else(|| Type::Variable(TypeVar::fresh()))
                        })
                        .collect();
                    Some((name.clone(), param_types))
                }
                _ => None,
            })
            .collect();

        // Store sigs in an outer scope (no local scope pushed yet).
        let mut outer_scope = HashMap::new();
        for (name, param_types) in &sigs {
            let return_type = Type::Variable(TypeVar::fresh());
            outer_scope.insert(
                name.clone(),
                Type::Function {
                    params: param_types.clone(),
                    return_type: Box::new(return_type),
                },
            );
        }
        self.scopes.push(outer_scope);

        // Pass 3: generate constraints for each function body.
        // Collect body-refs in a temp Vec to avoid simultaneous borrows.
        let body_refs: Vec<(&TypedExpr, usize)> = toplevels
            .iter()
            .enumerate()
            .filter_map(|(i, tl)| match tl {
                TopLevel::Defn { body, .. } => Some((body, i)),
                _ => None,
            })
            .collect();

        for (body, idx) in &body_refs {
            let tl = &toplevels[*idx];
            let name = match tl {
                TopLevel::Defn { name, .. } => name.clone(),
                _ => unreachable!(),
            };
            let params = match tl {
                TopLevel::Defn { parameters, .. } => parameters,
                _ => unreachable!(),
            };

            let sig = self.scope_lookup(&name).unwrap();
            let (declared_param_types, declared_return_type) = match &sig {
                Type::Function {
                    params,
                    return_type,
                } => (params.clone(), *return_type.clone()),
                _ => unreachable!(),
            };

            let mut local_scope = HashMap::new();
            for ((pname, _), pty) in params.iter().zip(declared_param_types.iter()) {
                local_scope.insert(pname.clone(), pty.clone());
            }
            self.scopes.push(local_scope);

            let body_type = self.infer_expr(body)?;
            self.scopes.pop();

            self.constrain(
                declared_return_type,
                body_type,
                &format!("return type of '{name}'"),
                0,
                0,
            );
        }

        self.scopes.pop(); // outer scope

        // Pass 4: unify and resolve
        if self.constraints.is_empty() {
            return Ok(());
        }
        let subst = Unifier::unify(&self.constraints)?;

        for tl in toplevels.iter_mut() {
            self.resolve_toplevel(tl, &subst);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // Expression inference
    // ------------------------------------------------------------------

    /// Infer the type of a [`TypedExpr`], generating constraints.
    /// Returns the (possibly-unresolved) type for immediate use.
    fn infer_expr(&mut self, expr: &TypedExpr) -> Result<Type, String> {
        match expr {
            // -- literals ---------------------------------------------------
            TypedExpr::Int(_, t) => {
                self.constrain(t.clone(), Type::Int64, "int literal", 0, 0);
                Ok(t.clone())
            }
            TypedExpr::Float(_, t) => {
                self.constrain(t.clone(), Type::Float64, "float literal", 0, 0);
                Ok(t.clone())
            }
            TypedExpr::Bool(_, t) => {
                self.constrain(t.clone(), Type::Bool, "bool literal", 0, 0);
                Ok(t.clone())
            }
            TypedExpr::Str(_, t) => {
                self.constrain(t.clone(), Type::String, "string literal", 0, 0);
                Ok(t.clone())
            }

            // -- variables --------------------------------------------------
            TypedExpr::Var(name, t) => {
                if let Some(found) = self.scope_lookup(name) {
                    self.constrain(t.clone(), found.clone(), &format!("variable '{name}'"), 0, 0);
                    Ok(t.clone())
                } else if let Some(kind) = builtin_kind(name) {
                    let builtin_ty = builtin_bare_type(&kind);
                    self.constrain(t.clone(), builtin_ty, &format!("builtin '{name}'"), 0, 0);
                    Ok(t.clone())
                } else {
                    Err(format!("Undefined variable: '{name}'"))
                }
            }

            // -- application ------------------------------------------------
            TypedExpr::App(func, args, ret_ty) => {
                let func_ty = self.infer_expr(func)?;
                let mut arg_types = Vec::new();
                for arg in args {
                    arg_types.push(self.infer_expr(arg)?);
                }

                // If the function is a builtin, handle with precise type rules.
                if let TypedExpr::Var(name, _) = func.as_ref() {
                    if let Some(kind) = builtin_kind(name) {
                        return self.infer_builtin_call(&kind, args, ret_ty);
                    }
                }

                self.constrain(
                    func_ty,
                    Type::Function {
                        params: arg_types,
                        return_type: Box::new(ret_ty.clone()),
                    },
                    "function application",
                    0,
                    0,
                );
                Ok(ret_ty.clone())
            }

            // -- let binding ------------------------------------------------
            TypedExpr::Let(bindings, body, body_ty) => {
                let mut local_scope = HashMap::new();
                for (name, val) in bindings {
                    let val_ty = self.infer_expr(val)?;
                    local_scope.insert(name.clone(), val_ty);
                }
                self.scopes.push(local_scope);
                let inferred = self.infer_expr(body)?;
                self.scopes.pop();
                self.constrain(body_ty.clone(), inferred, "let body", 0, 0);
                Ok(body_ty.clone())
            }

            // -- set! -------------------------------------------------------
            TypedExpr::Set(name, value, t) => {
                // Value must match the variable's declared type.
                let var_ty = self.scope_lookup(name).ok_or_else(|| {
                    format!("set!: unknown variable '{name}'")
                })?;
                let val_ty = self.infer_expr(value)?;
                self.constrain(var_ty, val_ty, &format!("set! '{name}'"), 0, 0);
                self.constrain(t.clone(), Type::Unit, "set! returns unit", 0, 0);
                Ok(t.clone())
            }

            // -- lambda -----------------------------------------------------
            TypedExpr::Lambda(params, body, fn_ty) => {
                let param_types: Vec<Type> = params
                    .iter()
                    .map(|(_, ty)| ty.clone().unwrap_or_else(|| Type::Variable(TypeVar::fresh())))
                    .collect();

                let mut local_scope = HashMap::new();
                for ((pname, _), pty) in params.iter().zip(param_types.iter()) {
                    local_scope.insert(pname.clone(), pty.clone());
                }
                self.scopes.push(local_scope);
                let body_type = self.infer_expr(body)?;
                self.scopes.pop();

                self.constrain(
                    fn_ty.clone(),
                    Type::Function {
                        params: param_types,
                        return_type: Box::new(body_type),
                    },
                    "lambda",
                    0,
                    0,
                );
                Ok(fn_ty.clone())
            }

            // -- if ---------------------------------------------------------
            TypedExpr::If(cond, then, else_branch, t) => {
                let cond_ty = self.infer_expr(cond)?;
                self.constrain(cond_ty, Type::Bool, "if condition", 0, 0);

                let then_ty = self.infer_expr(then)?;
                self.constrain(t.clone(), then_ty.clone(), "if then branch", 0, 0);

                if let Some(else_expr) = else_branch {
                    let else_ty = self.infer_expr(else_expr)?;
                    self.constrain(then_ty, else_ty, "if else branch", 0, 0);
                }

                Ok(t.clone())
            }

            // -- while ------------------------------------------------------
            TypedExpr::While(cond, body, t) => {
                let cond_ty = self.infer_expr(cond)?;
                self.constrain(cond_ty, Type::Bool, "while condition", 0, 0);
                self.infer_expr(body)?;
                self.constrain(t.clone(), Type::Unit, "while returns unit", 0, 0);
                Ok(t.clone())
            }

            // -- loop -------------------------------------------------------
            TypedExpr::Loop {
                variable,
                init,
                condition,
                step,
                body,
                ty,
            } => {
                let init_ty = self.infer_expr(init)?;
                let step_ty = self.infer_expr(step)?;
                // The loop variable type = init type = step type.
                self.constrain(
                    init_ty.clone(),
                    step_ty,
                    &format!("loop '{variable}' step"),
                    0,
                    0,
                );

                let cond_ty = self.infer_expr(condition)?;
                self.constrain(cond_ty, Type::Bool, "loop condition", 0, 0);

                // Variable is visible in body.
                let mut local_scope = HashMap::new();
                local_scope.insert(variable.clone(), init_ty);
                self.scopes.push(local_scope);
                self.infer_expr(body)?;
                self.scopes.pop();

                // Loop returns the final variable value.
                Ok(ty.clone())
            }

            // -- do ---------------------------------------------------------
            TypedExpr::Do(exprs, t) => {
                let mut last_ty = Type::Unit;
                for e in exprs {
                    last_ty = self.infer_expr(e)?;
                }
                self.constrain(t.clone(), last_ty, "do expression", 0, 0);
                Ok(t.clone())
            }

            // -- array literal ----------------------------------------------
            TypedExpr::Array(elements, t) => {
                if elements.is_empty() {
                    return Err("empty array literal is not supported".to_string());
                }
                let elem_ty = self.infer_expr(&elements[0])?;
                for elem in &elements[1..] {
                    let e_ty = self.infer_expr(elem)?;
                    self.constrain(elem_ty.clone(), e_ty, "array literal", 0, 0);
                }
                self.constrain(
                    t.clone(),
                    Type::Array(Box::new(elem_ty)),
                    "array literal type",
                    0,
                    0,
                );
                Ok(t.clone())
            }

            // -- new (allocation) -------------------------------------------
            TypedExpr::New {
                type_str,
                size_or_init,
                ty,
            } => {
                let inner_ty = parse_type_name(type_str);
                if let Some(init) = size_or_init {
                    // If it's an allocation with an initializer, constrain
                    // the initializer to be the size.
                    let init_ty = self.infer_expr(init)?;
                    self.constrain(init_ty, Type::Int64, "new size", 0, 0);
                }
                self.constrain(
                    ty.clone(),
                    Type::Pointer(Box::new(inner_ty)),
                    "new allocation type",
                    0,
                    0,
                );
                Ok(ty.clone())
            }

            // -- field access -----------------------------------------------
            TypedExpr::Field(object, field, t) => {
                let obj_ty = self.infer_expr(object)?;
                // We need the object to be a struct with this field.
                // Constrain through struct_fields lookup.
                // For inference, we create a placeholder — the actual
                // structural check happens at unification.
                if let Some(field_ty) = self.lookup_struct_field_type(&obj_ty, field) {
                    self.constrain(t.clone(), field_ty, &format!("field '{field}'"), 0, 0);
                } else {
                    // Fresh variable — we'll resolve it during unification.
                    self.constrain(t.clone(), Type::Variable(TypeVar::fresh()), "field access", 0, 0);
                }
                Ok(t.clone())
            }

            // -- set-field! -------------------------------------------------
            TypedExpr::SetField(object, field, value, t) => {
                let obj_ty = self.infer_expr(object)?;
                let val_ty = self.infer_expr(value)?;
                if let Some(field_ty) = self.lookup_struct_field_type(&obj_ty, field) {
                    self.constrain(val_ty, field_ty, &format!("set-field! '{field}'"), 0, 0);
                }
                self.constrain(t.clone(), Type::Unit, "set-field! returns unit", 0, 0);
                Ok(t.clone())
            }

            // -- index ------------------------------------------------------
            TypedExpr::Index(array, idx, t) => {
                let arr_ty = self.infer_expr(array)?;
                let idx_ty = self.infer_expr(idx)?;
                self.constrain(idx_ty, Type::Int64, "array index", 0, 0);
                self.constrain(
                    arr_ty,
                    Type::Array(Box::new(t.clone())),
                    "array index type",
                    0,
                    0,
                );
                Ok(t.clone())
            }

            // -- addr-of ----------------------------------------------------
            TypedExpr::AddrOf(operand, t) => {
                let op_ty = self.infer_expr(operand)?;
                self.constrain(
                    t.clone(),
                    Type::Pointer(Box::new(op_ty)),
                    "addr-of",
                    0,
                    0,
                );
                Ok(t.clone())
            }

            // -- deref ------------------------------------------------------
            TypedExpr::Deref(operand, t) => {
                let op_ty = self.infer_expr(operand)?;
                self.constrain(
                    op_ty,
                    Type::Pointer(Box::new(t.clone())),
                    "dereference",
                    0,
                    0,
                );
                Ok(t.clone())
            }
        }
    }

    // ------------------------------------------------------------------
    // Builtin calls
    // ------------------------------------------------------------------

    fn infer_builtin_call(
        &mut self,
        kind: &BuiltinKind,
        args: &[TypedExpr],
        ret_ty: &Type,
    ) -> Result<Type, String> {
        let arg_types: Result<Vec<_>, _> = args.iter().map(|a| self.infer_expr(a)).collect();
        let arg_types = arg_types?;

        match kind {
            BuiltinKind::Arith => {
                if arg_types.len() != 2 {
                    return Err(format!(
                        "arithmetic operator: expected 2 arguments, got {}",
                        arg_types.len()
                    ));
                }
                self.constrain(arg_types[0].clone(), arg_types[1].clone(), "arith args", 0, 0);
                self.constrain(ret_ty.clone(), arg_types[0].clone(), "arith result", 0, 0);
            }
            BuiltinKind::Cmp => {
                if arg_types.len() != 2 {
                    return Err(format!(
                        "comparison operator: expected 2 arguments, got {}",
                        arg_types.len()
                    ));
                }
                self.constrain(arg_types[0].clone(), arg_types[1].clone(), "cmp args", 0, 0);
                self.constrain(ret_ty.clone(), Type::Bool, "cmp result", 0, 0);
            }
            BuiltinKind::Logical => {
                for a in &arg_types {
                    self.constrain(a.clone(), Type::Bool, "logical arg", 0, 0);
                }
                self.constrain(ret_ty.clone(), Type::Bool, "logical result", 0, 0);
            }
            BuiltinKind::Not => {
                if arg_types.len() != 1 {
                    return Err(format!("!: expected 1 argument, got {}", arg_types.len()));
                }
                self.constrain(arg_types[0].clone(), Type::Bool, "! arg", 0, 0);
                self.constrain(ret_ty.clone(), Type::Bool, "! result", 0, 0);
            }
            BuiltinKind::Print => {
                if arg_types.len() < 1 {
                    return Err("print: expected at least 1 argument".to_string());
                }
                self.constrain(ret_ty.clone(), Type::Unit, "print result", 0, 0);
            }
            BuiltinKind::Malloc => {
                if arg_types.len() != 1 {
                    return Err(format!(
                        "malloc: expected 1 argument, got {}",
                        arg_types.len()
                    ));
                }
                self.constrain(arg_types[0].clone(), Type::Int64, "malloc size", 0, 0);
                let elem = Type::Variable(TypeVar::fresh());
                self.constrain(
                    ret_ty.clone(),
                    Type::Pointer(Box::new(elem)),
                    "malloc result",
                    0,
                    0,
                );
            }
            BuiltinKind::Free => {
                if arg_types.len() != 1 {
                    return Err(format!(
                        "free: expected 1 argument, got {}",
                        arg_types.len()
                    ));
                }
                let elem = Type::Variable(TypeVar::fresh());
                self.constrain(
                    arg_types[0].clone(),
                    Type::Pointer(Box::new(elem)),
                    "free arg",
                    0,
                    0,
                );
                self.constrain(ret_ty.clone(), Type::Unit, "free result", 0, 0);
            }
            BuiltinKind::SetIndex => {
                if arg_types.len() != 3 {
                    return Err(format!(
                        "aset!: expected 3 arguments, got {}",
                        arg_types.len()
                    ));
                }
                let elem = Type::Variable(TypeVar::fresh());
                self.constrain(
                    arg_types[0].clone(),
                    Type::Array(Box::new(elem.clone())),
                    "aset! array",
                    0,
                    0,
                );
                self.constrain(arg_types[1].clone(), Type::Int64, "aset! index", 0, 0);
                self.constrain(arg_types[2].clone(), elem, "aset! value", 0, 0);
                self.constrain(ret_ty.clone(), Type::Unit, "aset! result", 0, 0);
            }
        }

        Ok(ret_ty.clone())
    }

    // ------------------------------------------------------------------
    // Post-unification resolution
    // ------------------------------------------------------------------

    fn resolve_toplevel(&self, tl: &mut TopLevel, subst: &crate::middle_end::types::Substitution) {
        match tl {
            TopLevel::Defn { body, .. } => {
                self.resolve_expr(body, subst);
            }
            TopLevel::Struct { .. } => {}
        }
    }

    fn resolve_expr(&self, expr: &mut TypedExpr, subst: &crate::middle_end::types::Substitution) {
        let resolved = Unifier::resolve(subst, expr.get_type());
        *expr.get_type_mut() = resolved;

        match expr {
            TypedExpr::Int(..)
            | TypedExpr::Float(..)
            | TypedExpr::Bool(..)
            | TypedExpr::Str(..)
            | TypedExpr::Var(..) => {}

            TypedExpr::Let(bindings, body, _) => {
                for (_, val) in bindings.iter_mut() {
                    self.resolve_expr(val, subst);
                }
                self.resolve_expr(body, subst);
            }
            TypedExpr::Set(_, value, _) => self.resolve_expr(value, subst),

            TypedExpr::Lambda(_, body, _) => self.resolve_expr(body, subst),
            TypedExpr::App(func, args, _) => {
                self.resolve_expr(func, subst);
                for a in args.iter_mut() {
                    self.resolve_expr(a, subst);
                }
            }

            TypedExpr::If(cond, then, else_branch, _) => {
                self.resolve_expr(cond, subst);
                self.resolve_expr(then, subst);
                if let Some(e) = else_branch {
                    self.resolve_expr(e, subst);
                }
            }
            TypedExpr::While(cond, body, _) => {
                self.resolve_expr(cond, subst);
                self.resolve_expr(body, subst);
            }
            TypedExpr::Loop {
                init, condition, step, body, ..
            } => {
                self.resolve_expr(init, subst);
                self.resolve_expr(condition, subst);
                self.resolve_expr(step, subst);
                self.resolve_expr(body, subst);
            }
            TypedExpr::Do(exprs, _) => {
                for e in exprs.iter_mut() {
                    self.resolve_expr(e, subst);
                }
            }

            TypedExpr::Array(elements, _) => {
                for e in elements.iter_mut() {
                    self.resolve_expr(e, subst);
                }
            }
            TypedExpr::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    self.resolve_expr(init, subst);
                }
            }
            TypedExpr::Field(obj, _, _)
            | TypedExpr::SetField(obj, _, _, _)
            | TypedExpr::Index(obj, _, _) => {
                self.resolve_expr(obj, subst);
                if let TypedExpr::SetField(_, _, val, _) = expr {
                    self.resolve_expr(val, subst);
                }
                if let TypedExpr::Index(_, idx, _) = expr {
                    self.resolve_expr(idx, subst);
                }
            }
            TypedExpr::AddrOf(op, _) | TypedExpr::Deref(op, _) => {
                self.resolve_expr(op, subst);
            }
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn constrain(&mut self, lhs: Type, rhs: Type, context: &str, line: usize, column: usize) {
        self.constraints.push(Constraint {
            lhs,
            rhs,
            context: context.to_string(),
            line,
            column,
        });
    }

    fn scope_lookup(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }
        None // functions are also stored in an outer scope
    }

    fn scope_insert(&mut self, name: &String, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.clone(), ty);
        } else {
            // No scope yet — create one (used for function signature registration).
            let mut s = HashMap::new();
            s.insert(name.clone(), ty);
            self.scopes.push(s);
        }
    }

    fn lookup_struct_field_type(&self, obj_ty: &Type, field: &str) -> Option<Type> {
        // If the object type is a known struct, look up the field.
        match obj_ty {
            Type::Struct(name) => self
                .struct_fields
                .get(name)
                .and_then(|fields| {
                    fields
                        .iter()
                        .find(|(fname, _)| fname == field)
                        .map(|(_, fty)| fty.clone())
                }),
            Type::Variable(_) => None,
            _ => None,
        }
    }
}

/// Return a `Type` for a type-name string like "i64", "Point", "arr_i64".
fn parse_type_name(s: &str) -> Type {
    match s {
        "i64" => Type::Int64,
        "f64" => Type::Float64,
        "bool" => Type::Bool,
        "string" => Type::String,
        "unit" | "void" => Type::Unit,
        other if other.starts_with("arr_") => {
            Type::Array(Box::new(parse_type_name(&other[4..])))
        }
        other if other.starts_with("ptr_") => {
            Type::Pointer(Box::new(parse_type_name(&other[4..])))
        }
        other => Type::Struct(other.to_string()),
    }
}

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
            return_type: Box::new(Type::Unit),
        },
        BuiltinKind::SetIndex => {
            let elem = fresh();
            Type::Function {
                params: vec![
                    Type::Array(Box::new(elem.clone())),
                    Type::Int64,
                    elem,
                ],
                return_type: Box::new(Type::Unit),
            }
        }
    }
}
