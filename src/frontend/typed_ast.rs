//! Typed AST — a compiler IR where every expression node carries a resolved
//! [`Type`] field, bridging the purely syntactic [`SExpr`] layer to the
//! Hindley-Milner inference engine.
//!
//! # Two-phase construction
//!
//! 1. [`sexprs_to_toplevels`] converts a validated, macro-expanded
//!    `Vec<SExpr>` into a `Vec<TopLevel>`.  Every expression node starts
//!    with a fresh [`Type::Variable`] as its placeholder type.
//!
//! 2. [`crate::frontend::type_inferer::TypeInferer::infer_program`] walks
//!    this tree, generates Hindley-Milner constraints, unifies them, and
//!    **replaces** every placeholder with the fully resolved [`Type`].

use std::collections::HashMap;

use crate::frontend::sexpr::SExpr;
use crate::middle_end::types::{Type, TypeVar};

// ---------------------------------------------------------------------------
// Typed expression nodes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TypedExpr {
    // -- literals ---------------------------------------------------------
    Int(i64, Type),
    Float(f64, Type),
    Bool(bool, Type),
    Str(String, Type),

    // -- variables & bindings ---------------------------------------------
    Var(String, Type),
    Let(Vec<(String, TypedExpr)>, Box<TypedExpr>, Type),
    Set(String, Box<TypedExpr>, Type),

    // -- functions & application ------------------------------------------
    Lambda(Vec<(String, Option<Type>)>, Box<TypedExpr>, Type),
    App(Box<TypedExpr>, Vec<TypedExpr>, Type),

    // -- control flow -----------------------------------------------------
    If(Box<TypedExpr>, Box<TypedExpr>, Option<Box<TypedExpr>>, Type),
    While(Box<TypedExpr>, Box<TypedExpr>, Type),
    Loop {
        variable: String,
        init: Box<TypedExpr>,
        condition: Box<TypedExpr>,
        step: Box<TypedExpr>,
        body: Box<TypedExpr>,
        ty: Type,
    },
    Do(Vec<TypedExpr>, Type),

    // -- data structures --------------------------------------------------
    Array(Vec<TypedExpr>, Type),
    New {
        type_str: String,
        size_or_init: Option<Box<TypedExpr>>,
        ty: Type,
    },
    Field(Box<TypedExpr>, String, Type),
    SetField(Box<TypedExpr>, String, Box<TypedExpr>, Type),
    Index(Box<TypedExpr>, Box<TypedExpr>, Type),

    // -- pointer operations -----------------------------------------------
    AddrOf(Box<TypedExpr>, Type),
    Deref(Box<TypedExpr>, Type),
}

impl TypedExpr {
    /// Borrow the current type annotation.
    pub fn get_type(&self) -> &Type {
        match self {
            TypedExpr::Int(_, t)
            | TypedExpr::Float(_, t)
            | TypedExpr::Bool(_, t)
            | TypedExpr::Str(_, t)
            | TypedExpr::Var(_, t)
            | TypedExpr::Let(_, _, t)
            | TypedExpr::Set(_, _, t)
            | TypedExpr::Lambda(_, _, t)
            | TypedExpr::App(_, _, t)
            | TypedExpr::If(_, _, _, t)
            | TypedExpr::While(_, _, t)
            | TypedExpr::Loop { ty: t, .. }
            | TypedExpr::Do(_, t)
            | TypedExpr::Array(_, t)
            | TypedExpr::New { ty: t, .. }
            | TypedExpr::Field(_, _, t)
            | TypedExpr::SetField(_, _, _, t)
            | TypedExpr::Index(_, _, t)
            | TypedExpr::AddrOf(_, t)
            | TypedExpr::Deref(_, t) => t,
        }
    }

    /// Mutably borrow the type annotation (for the inferencer to replace
    /// type variables with resolved types).
    pub fn get_type_mut(&mut self) -> &mut Type {
        match self {
            TypedExpr::Int(_, t)
            | TypedExpr::Float(_, t)
            | TypedExpr::Bool(_, t)
            | TypedExpr::Str(_, t)
            | TypedExpr::Var(_, t)
            | TypedExpr::Let(_, _, t)
            | TypedExpr::Set(_, _, t)
            | TypedExpr::Lambda(_, _, t)
            | TypedExpr::App(_, _, t)
            | TypedExpr::If(_, _, _, t)
            | TypedExpr::While(_, _, t)
            | TypedExpr::Loop { ty: t, .. }
            | TypedExpr::Do(_, t)
            | TypedExpr::Array(_, t)
            | TypedExpr::New { ty: t, .. }
            | TypedExpr::Field(_, _, t)
            | TypedExpr::SetField(_, _, _, t)
            | TypedExpr::Index(_, _, t)
            | TypedExpr::AddrOf(_, t)
            | TypedExpr::Deref(_, t) => t,
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level definitions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TopLevel {
    Defn {
        name: String,
        /// (param_name, optional_type_annotation)
        parameters: Vec<(String, Option<Type>)>,
        body: TypedExpr,
        /// The declared or inferred return type.
        return_type: Type,
    },
    Struct {
        name: String,
        /// (field_name, field_type)
        fields: Vec<(String, Type)>,
    },
}

// ---------------------------------------------------------------------------
// Fresh type variable helpers
// ---------------------------------------------------------------------------

fn fresh() -> Type {
    Type::Variable(TypeVar::fresh())
}

// ---------------------------------------------------------------------------
// SExpr → TypedExpr  bridge
// ---------------------------------------------------------------------------

/// Convert a macro-expanded program (vector of top-level S-Expressions) into
/// a [`Vec<TopLevel>`], priming every expression node with a fresh type
/// variable.
///
/// Errors are reported for structurally invalid forms (wrong argument counts,
/// missing names, etc.) — pure syntactic validation, not type checking.
pub fn sexprs_to_toplevels(sexprs: Vec<SExpr>) -> Result<Vec<TopLevel>, String> {
    let mut struct_fields: HashMap<String, Vec<(String, Type)>> = HashMap::new();
    let mut toplevels = Vec::new();

    // Pass 1: collect struct definitions (needed by field / new resolution).
    for form in &sexprs {
        if let SExpr::List(items) = form {
            if items.first() == Some(&SExpr::Symbol("defstruct".to_string())) {
                if items.len() < 3 {
                    return Err("defstruct: expected (defstruct name [fields...])".to_string());
                }
                let name = sym_str(&items[1], "defstruct: name must be a symbol")?;
                let fields = match &items[2] {
                    SExpr::List(parts) => {
                        // Flatten the list: (name type name type ...)
                        let mut f = Vec::new();
                        for chunk in parts.chunks(2) {
                            if chunk.len() != 2 {
                                return Err(format!(
                                    "defstruct: expected (field_name type), got {:?}",
                                    chunk
                                ));
                            }
                            let fname = sym_str(&chunk[0], "defstruct: field name must be a symbol")?;
                            let fty = type_str_to_type(&chunk[1])?;
                            f.push((fname, fty));
                        }
                        f
                    }
                    other => {
                        return Err(format!(
                            "defstruct: expected field list, got {other}"
                        ));
                    }
                };
                struct_fields.insert(name.clone(), fields.clone());
                toplevels.push(TopLevel::Struct { name, fields });
            }
        }
    }

    // Pass 2: convert each form.
    for form in &sexprs {
        if let SExpr::List(items) = form {
            let head = &items[0];
            match head {
                SExpr::Symbol(s) if s == "defstruct" => continue, // already done
                SExpr::Symbol(s) if s == "defn" => {
                    toplevels.push(defn_from_sexpr(items, &struct_fields)?);
                }
                _ => {
                    // Expression at top level — wrap in an implicit do or
                    // just add directly.
                    let expr = expr_from_sexpr(form, &struct_fields)?;
                    // Wrap in a synthetic main function if it's a standalone expr.
                    // For simplicity, just add it as a toplevel with a generated name.
                    let body_ty = expr.get_type().clone();
                    toplevels.push(TopLevel::Defn {
                        name: "__toplevel__".to_string(),
                        parameters: vec![],
                        body: expr,
                        return_type: body_ty,
                    });
                }
            }
        } else {
            // Bare atom at top level (e.g. just "42") — also wrap.
            let expr = expr_from_sexpr(form, &struct_fields)?;
            let body_ty = expr.get_type().clone();
            toplevels.push(TopLevel::Defn {
                name: "__toplevel__".to_string(),
                parameters: vec![],
                body: expr,
                return_type: body_ty,
            });
        }
    }

    Ok(toplevels)
}

// ---------------------------------------------------------------------------
// defn from SExpr
// ---------------------------------------------------------------------------

fn defn_from_sexpr(
    items: &[SExpr],
    struct_fields: &HashMap<String, Vec<(String, Type)>>,
) -> Result<TopLevel, String> {
    if items.len() < 4 {
        return Err(format!(
            "defn: expected at least (defn name [params] body), got {} forms",
            items.len() - 1
        ));
    }

    let name = sym_str(&items[1], "defn: name must be a symbol")?;
    let params = extract_param_list(&items[2])?;

    // Multiple body forms are wrapped in an implicit `do`.
    let body = if items.len() == 4 {
        expr_from_sexpr(&items[3], struct_fields)?
    } else {
        let body_exprs: Result<Vec<_>, _> = items[3..]
            .iter()
            .map(|e| expr_from_sexpr(e, struct_fields))
            .collect();
        let body_exprs = body_exprs?;
        let ret_ty = body_exprs.last().map(|e| e.get_type().clone()).unwrap_or(fresh());
        TypedExpr::Do(body_exprs, ret_ty)
    };
    let return_type = body.get_type().clone();

    Ok(TopLevel::Defn {
        name,
        parameters: params,
        body,
        return_type,
    })
}

/// Extract (name, optional_type) from a parameter list like `[x y]` or `[x :i64 y]`.
fn extract_param_list(param_sexpr: &SExpr) -> Result<Vec<(String, Option<Type>)>, String> {
    let items = match param_sexpr {
        SExpr::List(items) => items,
        other => return Err(format!("defn: expected parameter list, got {other}")),
    };

    let mut params = Vec::new();
    let mut i = 0;
    while i < items.len() {
        let name = sym_str(&items[i], "defn: parameter name must be a symbol")?;
        // Check for a type annotation after a colon.
        if i + 2 < items.len() && items[i + 1] == SExpr::Symbol(":".to_string()) {
            let ty = type_str_to_type(&items[i + 2])?;
            params.push((name, Some(ty)));
            i += 3;
        } else {
            params.push((name, None));
            i += 1;
        }
    }

    Ok(params)
}

// ---------------------------------------------------------------------------
// Expression converter (SExpr → TypedExpr)
// ---------------------------------------------------------------------------

fn expr_from_sexpr(
    sexpr: &SExpr,
    struct_fields: &HashMap<String, Vec<(String, Type)>>,
) -> Result<TypedExpr, String> {
    match sexpr {
        SExpr::Symbol(s) => {
            // Recognize bool literals.
            if s == "true" {
                return Ok(TypedExpr::Bool(true, Type::Bool));
            }
            if s == "false" {
                return Ok(TypedExpr::Bool(false, Type::Bool));
            }
            Ok(TypedExpr::Var(s.clone(), fresh()))
        }

        SExpr::Integer(n) => Ok(TypedExpr::Int(*n, Type::Int64)),
        SExpr::Float(f) => Ok(TypedExpr::Float(*f, Type::Float64)),
        SExpr::String(s) => Ok(TypedExpr::Str(s.clone(), Type::String)),
        SExpr::Bool(b) => Ok(TypedExpr::Bool(*b, Type::Bool)),

        SExpr::List(items) => {
            if items.is_empty() {
                return Err("empty list is not a valid expression".to_string());
            }

            let head = &items[0];
            match head {
                // --- Special forms -------------------------------------------
                SExpr::Symbol(s) if s == "do" => {
                    let exprs: Result<Vec<_>, _> = items[1..]
                        .iter()
                        .map(|e| expr_from_sexpr(e, struct_fields))
                        .collect();
                    let exprs = exprs?;
                    let ty = exprs.last().map(|e| e.get_type().clone()).unwrap_or(fresh());
                    Ok(TypedExpr::Do(exprs, ty))
                }

                SExpr::Symbol(s) if s == "let" => {
                    if items.len() < 3 {
                        return Err("let: expected (let [bindings..] body)".to_string());
                    }
                    let bindings_sexpr = &items[1];
                    let binding_pairs = match bindings_sexpr {
                        SExpr::List(pairs) => pairs,
                        other => {
                            return Err(format!("let: expected binding list, got {other}"));
                        }
                    };
                    if binding_pairs.len() % 2 != 0 {
                        return Err(format!(
                            "let: bindings must be name+value pairs, got {} items",
                            binding_pairs.len()
                        ));
                    }

                    let mut bindings = Vec::new();
                    for chunk in binding_pairs.chunks(2) {
                        let bname =
                            sym_str(&chunk[0], "let: binding name must be a symbol")?;
                        let bval = expr_from_sexpr(&chunk[1], struct_fields)?;
                        bindings.push((bname, bval));
                    }

                    let body = if items.len() == 3 {
                        // Only one expression after bindings — implicit do.
                        expr_from_sexpr(&items[2], struct_fields)?
                    } else {
                        // Multiple body forms.
                        let body_exprs: Result<Vec<_>, _> = items[2..]
                            .iter()
                            .map(|e| expr_from_sexpr(e, struct_fields))
                            .collect();
                        let body_exprs = body_exprs?;
                        let ty = body_exprs
                            .last()
                            .map(|e| e.get_type().clone())
                            .unwrap_or(fresh());
                        TypedExpr::Do(body_exprs, ty)
                    };
                    let body_ty = body.get_type().clone();
                    Ok(TypedExpr::Let(bindings, Box::new(body), body_ty))
                }

                SExpr::Symbol(s) if s == "if" => {
                    if items.len() < 3 {
                        return Err("if: expected (if cond then [else])".to_string());
                    }
                    let cond = expr_from_sexpr(&items[1], struct_fields)?;
                    let then = expr_from_sexpr(&items[2], struct_fields)?;
                    let else_branch = if items.len() >= 4 {
                        Some(Box::new(expr_from_sexpr(&items[3], struct_fields)?))
                    } else {
                        None
                    };
                    let ret_ty = fresh();
                    Ok(TypedExpr::If(
                        Box::new(cond),
                        Box::new(then),
                        else_branch,
                        ret_ty,
                    ))
                }

                SExpr::Symbol(s) if s == "while" => {
                    if items.len() != 3 {
                        return Err("while: expected (while cond body)".to_string());
                    }
                    let cond = expr_from_sexpr(&items[1], struct_fields)?;
                    let body = expr_from_sexpr(&items[2], struct_fields)?;
                    Ok(TypedExpr::While(
                        Box::new(cond),
                        Box::new(body),
                        Type::Unit,
                    ))
                }

                SExpr::Symbol(s) if s == "loop" => {
                    // Support two loop syntaxes:
                    //   Old: (loop [var init] cond step body)
                    //   New: (loop [var init cond step] body)
                    if items.len() < 3 {
                        return Err("loop: expected (loop header body)".to_string());
                    }
                    let loop_header = match &items[1] {
                        SExpr::List(h) => h,
                        other => {
                            return Err(format!("loop: expected loop header list, got {other}"));
                        }
                    };
                    let (variable, init, condition, step, body) = if loop_header.len() == 4 {
                        // New syntax: (loop [var init cond step] body)
                        let variable =
                            sym_str(&loop_header[0], "loop: variable must be a symbol")?;
                        let init = expr_from_sexpr(&loop_header[1], struct_fields)?;
                        let condition = expr_from_sexpr(&loop_header[2], struct_fields)?;
                        let step = expr_from_sexpr(&loop_header[3], struct_fields)?;
                        let body = expr_from_sexpr(&items[2], struct_fields)?;
                        (variable, init, condition, step, body)
                    } else if loop_header.len() == 2 {
                        // Old syntax: (loop [var init] cond step body)
                        if items.len() < 5 {
                            return Err(format!(
                                "loop: old syntax expects (loop [var init] cond step body), got {} forms",
                                items.len() - 1
                            ));
                        }
                        let variable =
                            sym_str(&loop_header[0], "loop: variable must be a symbol")?;
                        let init = expr_from_sexpr(&loop_header[1], struct_fields)?;
                        let condition = expr_from_sexpr(&items[2], struct_fields)?;
                        let step = expr_from_sexpr(&items[3], struct_fields)?;
                        let body = expr_from_sexpr(&items[4], struct_fields)?;
                        (variable, init, condition, step, body)
                    } else {
                        return Err(format!(
                            "loop: header must have 4 elements (new syntax) or 2 (old syntax), got {}",
                            loop_header.len()
                        ));
                    };
                    let ret_ty = fresh();
                    Ok(TypedExpr::Loop {
                        variable,
                        init: Box::new(init),
                        condition: Box::new(condition),
                        step: Box::new(step),
                        body: Box::new(body),
                        ty: ret_ty,
                    })
                }

                SExpr::Symbol(s) if s == "lambda" => {
                    if items.len() < 3 {
                        return Err("lambda: expected (lambda [params] body)".to_string());
                    }
                    let params = extract_param_list(&items[1])?;
                    let body = expr_from_sexpr(&items[2], struct_fields)?;
                    let fn_ty = fresh(); // will be constrained to Function type
                    Ok(TypedExpr::Lambda(params, Box::new(body), fn_ty))
                }

                SExpr::Symbol(s) if s == "set!" => {
                    if items.len() != 3 {
                        return Err("set!: expected (set! name value)".to_string());
                    }
                    let name = sym_str(&items[1], "set!: variable name must be a symbol")?;
                    let value = expr_from_sexpr(&items[2], struct_fields)?;
                    Ok(TypedExpr::Set(name, Box::new(value), Type::Unit))
                }

                SExpr::Symbol(s) if s == "new" => {
                    if items.len() < 2 {
                        return Err("new: expected (new type [size])".to_string());
                    }
                    let type_str = sym_str(&items[1], "new: type must be a symbol or string")?;
                    let size_or_init = if items.len() >= 3 {
                        Some(Box::new(expr_from_sexpr(&items[2], struct_fields)?))
                    } else {
                        None
                    };
                    let ret_ty = fresh();
                    Ok(TypedExpr::New {
                        type_str,
                        size_or_init,
                        ty: ret_ty,
                    })
                }

                SExpr::Symbol(s) if s == "&" => {
                    if items.len() != 2 {
                        return Err("&: expected (& operand)".to_string());
                    }
                    let operand = expr_from_sexpr(&items[1], struct_fields)?;
                    let ret_ty = fresh();
                    Ok(TypedExpr::AddrOf(Box::new(operand), ret_ty))
                }

                SExpr::Symbol(s) if s == "*" => {
                    // Dereference — only if it appears as (operand) or (* operand).
                    // In SExpr form, `*` as a function is `(* x)`.
                    if items.len() != 2 {
                        // Could be multiplication (binary), which is a regular call.
                        return app_from_sexpr(sexpr, struct_fields);
                    }
                    let operand = expr_from_sexpr(&items[1], struct_fields)?;
                    let ret_ty = fresh();
                    Ok(TypedExpr::Deref(Box::new(operand), ret_ty))
                }

                SExpr::Symbol(s) if s == "field" || s == ".-" => {
                    if items.len() != 3 {
                        return Err(format!("{s}: expected ({s} object field_name)"));
                    }
                    let object = expr_from_sexpr(&items[1], struct_fields)?;
                    let field = sym_str(&items[2], "field: field name must be a symbol")?;
                    let ret_ty = fresh();
                    Ok(TypedExpr::Field(Box::new(object), field, ret_ty))
                }

                SExpr::Symbol(s) if s == "set-field!" => {
                    if items.len() != 4 {
                        return Err("set-field!: expected (set-field! object field value)".to_string());
                    }
                    let object = expr_from_sexpr(&items[1], struct_fields)?;
                    let field = sym_str(&items[2], "set-field!: field name must be a symbol")?;
                    let value = expr_from_sexpr(&items[3], struct_fields)?;
                    Ok(TypedExpr::SetField(
                        Box::new(object),
                        field,
                        Box::new(value),
                        Type::Unit,
                    ))
                }

                SExpr::Symbol(s) if s == "index" => {
                    if items.len() != 3 {
                        return Err("index: expected (index array idx)".to_string());
                    }
                    let array = expr_from_sexpr(&items[1], struct_fields)?;
                    let idx = expr_from_sexpr(&items[2], struct_fields)?;
                    let ret_ty = fresh();
                    Ok(TypedExpr::Index(Box::new(array), Box::new(idx), ret_ty))
                }

                SExpr::Symbol(s) if s == "aset!" => {
                    // (aset! array index value) — implemented as a builtin call.
                    return app_from_sexpr(sexpr, struct_fields);
                }

                // --- Array literal ----------------------------------------
                SExpr::Symbol(s) if s == "array" || s == "arr" => {
                    let elements: Result<Vec<_>, _> = items[1..]
                        .iter()
                        .map(|e| expr_from_sexpr(e, struct_fields))
                        .collect();
                    let elements = elements?;
                    let elem_ty = fresh();
                    let array_ty = Type::Array(Box::new(elem_ty));
                    Ok(TypedExpr::Array(elements, array_ty))
                }

                // --- Regular function application ---------------------------
                _ => app_from_sexpr(sexpr, struct_fields),
            }
        }
    }
}

/// Convert a list SExpr to a function application.
fn app_from_sexpr(
    sexpr: &SExpr,
    struct_fields: &HashMap<String, Vec<(String, Type)>>,
) -> Result<TypedExpr, String> {
    match sexpr {
        SExpr::List(items) => {
            let func = expr_from_sexpr(&items[0], struct_fields)?;
            let args: Result<Vec<_>, _> = items[1..]
                .iter()
                .map(|e| expr_from_sexpr(e, struct_fields))
                .collect();
            let args = args?;
            let ret_ty = fresh();
            Ok(TypedExpr::App(Box::new(func), args, ret_ty))
        }
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sym_str(sexpr: &SExpr, msg: &str) -> Result<String, String> {
    match sexpr {
        SExpr::Symbol(s) => Ok(s.clone()),
        other => Err(format!("{msg}, got {other}")),
    }
}

/// Convert an SExpr representing a type annotation into a [`Type`].
fn type_str_to_type(sexpr: &SExpr) -> Result<Type, String> {
    match sexpr {
        SExpr::Symbol(s) => match s.as_str() {
            "i64" => Ok(Type::Int64),
            "f64" => Ok(Type::Float64),
            "bool" => Ok(Type::Bool),
            "string" => Ok(Type::String),
            "unit" | "void" => Ok(Type::Unit),
            other if other.starts_with("arr_") || other.starts_with("ptr_") => {
                let inner = &other[4..];
                let inner_ty = type_str_to_type(&SExpr::Symbol(inner.to_string()))?;
                if other.starts_with("arr_") {
                    Ok(Type::Array(Box::new(inner_ty)))
                } else {
                    Ok(Type::Pointer(Box::new(inner_ty)))
                }
            }
            other => Ok(Type::Struct(other.to_string())),
        },
        SExpr::List(items) if items.len() == 2 => {
            // (-> param return) syntax
            let ret = type_str_to_type(&items[1])?;
            match &items[0] {
                SExpr::List(params) => {
                    let p: Result<Vec<_>, _> =
                        params.iter().map(type_str_to_type).collect();
                    Ok(Type::Function {
                        params: p?,
                        return_type: Box::new(ret),
                    })
                }
                SExpr::Symbol(s) if s == "->" => {
                    // Single-param function
                    Ok(Type::Function {
                        params: vec![],
                        return_type: Box::new(ret),
                    })
                }
                _ => Err(format!("invalid function type syntax: {sexpr}")),
            }
        }
        SExpr::List(items) if items.len() >= 3 && items[0] == SExpr::Symbol("fn".to_string()) => {
            let p: Result<Vec<_>, _> = items[1..items.len() - 1]
                .iter()
                .map(type_str_to_type)
                .collect();
            let ret = type_str_to_type(&items[items.len() - 1])?;
            Ok(Type::Function {
                params: p?,
                return_type: Box::new(ret),
            })
        }
        other => Err(format!("invalid type syntax: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn type_check_roundtrip(source: &str) -> Vec<TopLevel> {
        let sexprs = crate::frontend::sexpr::read_source(source)
            .expect("reader should succeed");
        let mut toplevels = sexprs_to_toplevels(sexprs)
            .expect("conversion to typed AST should succeed");
        // Run inference to resolve types.
        let mut inferer = crate::frontend::type_inferer::TypeInferer::new();
        inferer.infer_program(&mut toplevels)
            .expect("type inference should succeed");
        toplevels
    }

    fn check_type(source: &str, expected: Type) {
        let toplevels = type_check_roundtrip(source);
        match &toplevels[0] {
            TopLevel::Defn { body, return_type, .. } => {
                let resolved = crate::middle_end::unification::Unifier::resolve(
                    &Default::default(),
                    return_type,
                );
                assert_eq!(resolved, expected, "return type mismatch");
            }
            _ => panic!("expected Defn"),
        }
    }

    #[test]
    fn literal_int_is_int64() {
        check_type("42", Type::Int64);
    }

    #[test]
    fn literal_float_is_float64() {
        check_type("3.14", Type::Float64);
    }

    #[test]
    fn literal_bool_is_bool() {
        check_type("true", Type::Bool);
        check_type("false", Type::Bool);
    }

    #[test]
    fn addition_infers_int64() {
        check_type("(+ 1 2)", Type::Int64);
    }

    #[test]
    fn if_branches_must_agree() {
        check_type("(if true 1 0)", Type::Int64);
    }
}
