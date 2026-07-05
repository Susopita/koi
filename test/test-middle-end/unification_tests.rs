use koi::middle_end::types::{Constraint, Substitution, Type, TypeVar};
use koi::middle_end::unification::Unifier;

fn constraint(lhs: Type, rhs: Type) -> Constraint {
    Constraint {
        lhs,
        rhs,
        context: "test".to_string(),
        line: 1,
        column: 1,
    }
}

#[test]
fn unify_binds_a_variable_to_a_concrete_type() {
    let v = TypeVar::fresh();
    let subst = Unifier::unify(&[constraint(Type::Variable(v), Type::Int64)]).unwrap();
    assert_eq!(subst.apply(&Type::Variable(v)), Type::Int64);
}

#[test]
fn unify_chains_two_variables_together() {
    let a = TypeVar::fresh();
    let b = TypeVar::fresh();
    let subst = Unifier::unify(&[
        constraint(Type::Variable(a), Type::Variable(b)),
        constraint(Type::Variable(b), Type::Int64),
    ])
    .unwrap();
    assert_eq!(subst.apply(&Type::Variable(a)), Type::Int64);
}

#[test]
fn unify_same_concrete_types_succeeds() {
    assert!(Unifier::unify(&[constraint(Type::Int64, Type::Int64)]).is_ok());
    assert!(Unifier::unify(&[constraint(Type::Bool, Type::Bool)]).is_ok());
    assert!(Unifier::unify(&[constraint(Type::Float64, Type::Float64)]).is_ok());
    assert!(Unifier::unify(&[constraint(Type::String, Type::String)]).is_ok());
}

#[test]
fn unify_unit_with_itself_succeeds() {
    assert!(Unifier::unify(&[constraint(Type::Unit, Type::Unit)]).is_ok());
}

#[test]
fn unify_unit_with_a_concrete_type_fails() {
    let err = Unifier::unify(&[constraint(Type::Unit, Type::Int64)]).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn unify_mismatched_concrete_types_fails() {
    let err = Unifier::unify(&[constraint(Type::Int64, Type::Bool)]).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
    assert!(err.contains("line 1"), "expected location info: {err}");
}

#[test]
fn unify_same_named_structs_succeeds() {
    assert!(
        Unifier::unify(&[constraint(
            Type::Struct("Point".to_string()),
            Type::Struct("Point".to_string())
        )])
        .is_ok()
    );
}

#[test]
fn unify_differently_named_structs_fails() {
    let err = Unifier::unify(&[constraint(
        Type::Struct("Point".to_string()),
        Type::Struct("Pair".to_string()),
    )])
    .unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn unify_recurses_into_arrays_and_pointers() {
    let v = TypeVar::fresh();
    let subst = Unifier::unify(&[constraint(
        Type::Array(Box::new(Type::Variable(v))),
        Type::Array(Box::new(Type::Int64)),
    )])
    .unwrap();
    assert_eq!(subst.apply(&Type::Variable(v)), Type::Int64);

    let p = TypeVar::fresh();
    let subst = Unifier::unify(&[constraint(
        Type::Pointer(Box::new(Type::Variable(p))),
        Type::Pointer(Box::new(Type::Bool)),
    )])
    .unwrap();
    assert_eq!(subst.apply(&Type::Variable(p)), Type::Bool);
}

#[test]
fn unify_function_types_matches_params_and_return() {
    let p = TypeVar::fresh();
    let r = TypeVar::fresh();
    let lhs = Type::Function {
        params: vec![Type::Variable(p)],
        return_type: Box::new(Type::Variable(r)),
    };
    let rhs = Type::Function {
        params: vec![Type::Int64],
        return_type: Box::new(Type::Bool),
    };
    let subst = Unifier::unify(&[constraint(lhs, rhs)]).unwrap();
    assert_eq!(subst.apply(&Type::Variable(p)), Type::Int64);
    assert_eq!(subst.apply(&Type::Variable(r)), Type::Bool);
}

#[test]
fn unify_function_arity_mismatch_fails() {
    let lhs = Type::Function {
        params: vec![Type::Int64],
        return_type: Box::new(Type::Int64),
    };
    let rhs = Type::Function {
        params: vec![Type::Int64, Type::Int64],
        return_type: Box::new(Type::Int64),
    };
    let err = Unifier::unify(&[constraint(lhs, rhs)]).unwrap_err();
    assert!(err.contains("arity mismatch"), "unexpected error: {err}");
}

#[test]
fn unify_occurs_check_rejects_infinite_types() {
    let v = TypeVar::fresh();
    let err = Unifier::unify(&[constraint(
        Type::Variable(v),
        Type::Array(Box::new(Type::Variable(v))),
    )])
    .unwrap_err();
    assert!(err.contains("Infinite type"), "unexpected error: {err}");
}

#[test]
fn resolve_defaults_an_unbound_variable_to_int64() {
    let v = TypeVar::fresh();
    let subst = Substitution::new();
    assert_eq!(Unifier::resolve(&subst, &Type::Variable(v)), Type::Int64);
}

#[test]
fn resolve_defaults_variables_nested_inside_compound_types() {
    let v = TypeVar::fresh();
    let subst = Substitution::new();
    assert_eq!(
        Unifier::resolve(&subst, &Type::Array(Box::new(Type::Variable(v)))),
        Type::Array(Box::new(Type::Int64))
    );
    assert_eq!(
        Unifier::resolve(
            &subst,
            &Type::Function {
                params: vec![Type::Variable(v)],
                return_type: Box::new(Type::Variable(v))
            }
        ),
        Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn resolve_leaves_already_concrete_types_untouched() {
    let subst = Substitution::new();
    assert_eq!(
        Unifier::resolve(&subst, &Type::Struct("Point".to_string())),
        Type::Struct("Point".to_string())
    );
    assert_eq!(Unifier::resolve(&subst, &Type::Bool), Type::Bool);
}

#[test]
fn resolve_leaves_unit_untouched() {
    let subst = Substitution::new();
    assert_eq!(Unifier::resolve(&subst, &Type::Unit), Type::Unit);
}
