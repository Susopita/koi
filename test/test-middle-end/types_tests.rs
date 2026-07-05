use koi::middle_end::types::{Substitution, Type, TypeVar};

#[test]
fn fresh_type_vars_have_distinct_ids() {
    let a = TypeVar::fresh();
    let b = TypeVar::fresh();
    assert_ne!(a, b);
}

#[test]
fn apply_resolves_a_bound_variable() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    subst.bind(v, Type::Int64).unwrap();

    assert_eq!(subst.apply(&Type::Variable(v)), Type::Int64);
}

#[test]
fn apply_resolves_transitively_through_a_chain() {
    let a = TypeVar::fresh();
    let b = TypeVar::fresh();
    let mut subst = Substitution::new();
    subst.bind(a, Type::Variable(b)).unwrap();
    subst.bind(b, Type::Bool).unwrap();

    assert_eq!(subst.apply(&Type::Variable(a)), Type::Bool);
}

#[test]
fn apply_leaves_an_unbound_variable_unchanged() {
    let v = TypeVar::fresh();
    let subst = Substitution::new();
    assert_eq!(subst.apply(&Type::Variable(v)), Type::Variable(v));
}

#[test]
fn apply_recurses_into_compound_types() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    subst.bind(v, Type::Int64).unwrap();

    assert_eq!(
        subst.apply(&Type::Array(Box::new(Type::Variable(v)))),
        Type::Array(Box::new(Type::Int64))
    );
    assert_eq!(
        subst.apply(&Type::Pointer(Box::new(Type::Variable(v)))),
        Type::Pointer(Box::new(Type::Int64))
    );
    assert_eq!(
        subst.apply(&Type::Function {
            params: vec![Type::Variable(v)],
            return_type: Box::new(Type::Variable(v))
        }),
        Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn bind_accepts_a_normal_binding() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    assert!(subst.bind(v, Type::String).is_ok());
}

#[test]
fn bind_rejects_direct_self_reference() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    assert!(subst.bind(v, Type::Variable(v)).is_err());
}

#[test]
fn bind_rejects_an_indirect_cycle() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    let cyclic = Type::Array(Box::new(Type::Variable(v)));
    assert!(subst.bind(v, cyclic).is_err());
}

#[test]
fn bind_rejects_a_cycle_through_a_function_type() {
    let v = TypeVar::fresh();
    let mut subst = Substitution::new();
    let cyclic = Type::Function {
        params: vec![Type::Int64],
        return_type: Box::new(Type::Variable(v)),
    };
    assert!(subst.bind(v, cyclic).is_err());
}

#[test]
fn mangled_name_covers_every_type_variant() {
    assert_eq!(Type::Int64.mangled_name(), "i64");
    assert_eq!(Type::Float64.mangled_name(), "f64");
    assert_eq!(Type::Bool.mangled_name(), "bool");
    assert_eq!(Type::String.mangled_name(), "string");
    assert_eq!(Type::Array(Box::new(Type::Int64)).mangled_name(), "arr_i64");
    assert_eq!(
        Type::Pointer(Box::new(Type::Int64)).mangled_name(),
        "ptr_i64"
    );
    assert_eq!(Type::Struct("Point".to_string()).mangled_name(), "Point");
    assert_eq!(
        Type::Function {
            params: vec![Type::Int64, Type::Int64],
            return_type: Box::new(Type::Int64)
        }
        .mangled_name(),
        "fn_i64_i64_to_i64"
    );
    assert_eq!(Type::Variable(TypeVar { id: 7 }).mangled_name(), "T7");
    assert_eq!(Type::Unit.mangled_name(), "unit");
}

#[test]
fn apply_leaves_unit_unchanged() {
    let subst = Substitution::new();
    assert_eq!(subst.apply(&Type::Unit), Type::Unit);
}
