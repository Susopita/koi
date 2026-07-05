use crate::middle_end::types::{Constraint, Substitution, Type};

pub struct Unifier;

impl Unifier {
    pub fn unify(constraints: &[Constraint]) -> Result<Substitution, String> {
        let mut subst = Substitution::new();

        for constraint in constraints {
            let lhs = subst.apply(&constraint.lhs);
            let rhs = subst.apply(&constraint.rhs);

            Self::unify_types(&mut subst, &lhs, &rhs).map_err(|e| {
                format!(
                    "{} at line {}, column {} ({})",
                    e, constraint.line, constraint.column, constraint.context
                )
            })?;
        }

        Ok(subst)
    }

    fn unify_types(subst: &mut Substitution, lhs: &Type, rhs: &Type) -> Result<(), String> {
        match (lhs, rhs) {
            (Type::Int64, Type::Int64) => Ok(()),
            (Type::Float64, Type::Float64) => Ok(()),
            (Type::Bool, Type::Bool) => Ok(()),
            (Type::String, Type::String) => Ok(()),
            (Type::Unit, Type::Unit) => Ok(()),
            (Type::Struct(a), Type::Struct(b)) if a == b => Ok(()),

            (Type::Variable(v), t) | (t, Type::Variable(v)) => {
                if let Type::Variable(v2) = t
                    && v == v2
                {
                    return Ok(());
                }
                subst.bind(*v, t.clone())
            }

            (Type::Array(a), Type::Array(b)) => Self::unify_types(subst, a, b),
            (Type::Pointer(a), Type::Pointer(b)) => Self::unify_types(subst, a, b),

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
                    let a = subst.apply(a);
                    let b = subst.apply(b);
                    Self::unify_types(subst, &a, &b)?;
                }

                let r1 = subst.apply(r1);
                let r2 = subst.apply(r2);
                Self::unify_types(subst, &r1, &r2)
            }

            _ => Err(format!("Type mismatch: {:?} vs {:?}", lhs, rhs)),
        }
    }

    /// After unification, any type variable that never got bound to
    /// anything concrete (e.g. an unused parameter, or a genuinely
    /// unconstrained literal) defaults to `Int64` -- matching this
    /// language's "everything is 64-bit" MVP convention, and guaranteeing
    /// the emitted IR never contains an unresolved `?T`.
    pub fn resolve(subst: &Substitution, ty: &Type) -> Type {
        match subst.apply(ty) {
            Type::Variable(_) => Type::Int64,
            Type::Array(elem) => Type::Array(Box::new(Self::resolve(subst, &elem))),
            Type::Pointer(elem) => Type::Pointer(Box::new(Self::resolve(subst, &elem))),
            Type::Function {
                params,
                return_type,
            } => Type::Function {
                params: params.iter().map(|p| Self::resolve(subst, p)).collect(),
                return_type: Box::new(Self::resolve(subst, &return_type)),
            },
            other => other,
        }
    }
}
