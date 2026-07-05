use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    /// The type of expressions run only for their side effects (`set!`,
    /// `while`) -- carries no value and no substructure, so every generic
    /// recursive helper below (`apply`, `occurs_check`, `resolve`) handles it
    /// correctly via their existing catch-all arms.
    Unit,
}

impl Type {
    /// Human-readable name for the MVP's primitive type annotations
    /// ("i64", "f64", ...) and for name-mangling in monomorphization.
    pub fn mangled_name(&self) -> String {
        match self {
            Type::Int64 => "i64".to_string(),
            Type::Float64 => "f64".to_string(),
            Type::Bool => "bool".to_string(),
            Type::String => "string".to_string(),
            Type::Array(elem) => format!("arr_{}", elem.mangled_name()),
            Type::Pointer(elem) => format!("ptr_{}", elem.mangled_name()),
            Type::Struct(name) => name.clone(),
            Type::Function {
                params,
                return_type,
            } => {
                let param_names: Vec<String> = params.iter().map(Type::mangled_name).collect();
                format!(
                    "fn_{}_to_{}",
                    param_names.join("_"),
                    return_type.mangled_name()
                )
            }
            Type::Variable(v) => format!("T{}", v.id),
            Type::Unit => "unit".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypeVar {
    pub id: usize,
}

static TYPE_VAR_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl TypeVar {
    /// Safe replacement for the spec's `static mut` counter: an `AtomicUsize`
    /// gives the same "call it from anywhere, get a fresh id" ergonomics
    /// without `unsafe`.
    pub fn fresh() -> Self {
        TypeVar {
            id: TYPE_VAR_COUNTER.fetch_add(1, Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Constraint {
    pub lhs: Type,
    pub rhs: Type,
    pub context: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Default)]
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
            Type::Function {
                params,
                return_type,
            } => Type::Function {
                params: params.iter().map(|p| self.apply(p)).collect(),
                return_type: Box::new(self.apply(return_type)),
            },
            _ => ty.clone(),
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
            Type::Function {
                params,
                return_type,
            } => {
                params.iter().any(|p| self.occurs_check(var, p))
                    || self.occurs_check(var, return_type)
            }
            _ => false,
        }
    }
}
