#[path = "support.rs"]
mod support;
use support::*;

use koi::frontend::ast::ASTNode;
use koi::middle_end::inference::ConstraintGenerator;
use koi::middle_end::types::Type;
use koi::middle_end::unification::Unifier;
use std::collections::HashMap;

/// Runs inference + unification + resolution together, since that's how
/// they're actually used -- constraint generation alone doesn't reject
/// anything, it's unification that catches type errors.
fn infer(prog: &ASTNode) -> Result<HashMap<String, Type>, String> {
    let mut generator = ConstraintGenerator::new();
    generator.generate_program(prog)?;
    let subst = Unifier::unify(generator.constraints())?;
    Ok(generator
        .functions()
        .iter()
        .map(|(k, v)| (k.clone(), Unifier::resolve(&subst, v)))
        .collect())
}

fn fn_type(functions: &HashMap<String, Type>, name: &str) -> Type {
    functions
        .get(name)
        .unwrap_or_else(|| panic!("no signature recorded for '{name}'"))
        .clone()
}

#[test]
fn add_like_function_infers_int64_params_and_return() {
    let prog = program(vec![
        defn(
            "add",
            vec![("x", None), ("y", None)],
            call_named("+", vec![var("x"), var("y")]),
        ),
        defn("main", vec![], call_named("add", vec![int(5), int(3)])),
    ]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "add"),
        Type::Function {
            params: vec![Type::Int64, Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn recursive_function_infers_correctly() {
    let prog = program(vec![defn(
        "fib",
        vec![("n", None)],
        if_expr(
            call_named("<=", vec![var("n"), int(1)]),
            var("n"),
            Some(call_named(
                "+",
                vec![
                    call_named("fib", vec![call_named("-", vec![var("n"), int(1)])]),
                    call_named("fib", vec![call_named("-", vec![var("n"), int(2)])]),
                ],
            )),
        ),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "fib"),
        Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn forward_reference_between_functions_resolves() {
    let prog = program(vec![
        defn("a", vec![], call_named("b", vec![])),
        defn("b", vec![], int(1)),
    ]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "a"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int64)
        }
    );
    assert_eq!(
        fn_type(&functions, "b"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn builtin_comparison_returns_bool() {
    let prog = program(vec![defn(
        "cmp",
        vec![("x", None), ("y", None)],
        call_named("<", vec![var("x"), var("y")]),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "cmp"),
        Type::Function {
            params: vec![Type::Int64, Type::Int64],
            return_type: Box::new(Type::Bool)
        }
    );
}

#[test]
fn builtin_logical_forces_bool_operands() {
    let prog = program(vec![defn(
        "land",
        vec![("a", None), ("b", None)],
        call_named("&&", vec![var("a"), var("b")]),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "land"),
        Type::Function {
            params: vec![Type::Bool, Type::Bool],
            return_type: Box::new(Type::Bool)
        }
    );
}

#[test]
fn let_binding_is_sequential() {
    let prog = program(vec![defn(
        "f",
        vec![],
        let_binding(
            vec![
                ("a", int(1)),
                ("b", call_named("+", vec![var("a"), int(1)])),
            ],
            var("b"),
        ),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn if_without_else_unifies_condition_and_branch_when_they_share_a_variable() {
    let prog = program(vec![defn(
        "f",
        vec![("x", None)],
        if_expr(var("x"), var("x"), None),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![Type::Bool],
            return_type: Box::new(Type::Bool)
        }
    );
}

#[test]
fn lambda_as_a_return_value_infers_a_function_type() {
    let prog = program(vec![defn(
        "use-lambda",
        vec![],
        lambda(vec![("y", None)], call_named("+", vec![var("y"), int(1)])),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "use-lambda"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Function {
                params: vec![Type::Int64],
                return_type: Box::new(Type::Int64)
            })
        }
    );
}

#[test]
fn struct_field_access_resolves_via_unique_field_name() {
    let prog = program(vec![
        defstruct("Point", vec![("x", "i64"), ("y", "i64")]),
        defn("get-x", vec![("p", None)], field_access(var("p"), "x")),
    ]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "get-x"),
        Type::Function {
            params: vec![Type::Struct("Point".to_string())],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn set_field_yields_unit_and_constrains_value_to_the_field_type() {
    let prog = program(vec![
        defstruct("Point", vec![("x", "i64"), ("y", "i64")]),
        defn(
            "set-x",
            vec![("p", None), ("v", None)],
            set_field(var("p"), "x", var("v")),
        ),
    ]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "set-x"),
        Type::Function {
            params: vec![Type::Struct("Point".to_string()), Type::Int64],
            return_type: Box::new(Type::Unit),
        }
    );
}

#[test]
fn set_field_wrong_value_type_is_rejected() {
    let prog = program(vec![
        defstruct("Point", vec![("x", "i64"), ("y", "i64")]),
        defn(
            "bad-set-x",
            vec![("p", None)],
            set_field(var("p"), "x", string_lit("oops")),
        ),
    ]);
    assert!(infer(&prog).is_err());
}

#[test]
fn ambiguous_field_name_falls_back_without_erroring() {
    let prog = program(vec![
        defstruct("A", vec![("x", "i64")]),
        defstruct("B", vec![("x", "f64")]),
        defn("get-x", vec![("p", None)], field_access(var("p"), "x")),
    ]);
    // Two structs both define `x` -- the name doesn't uniquely identify one,
    // so this must not error even though it can't pick a concrete struct.
    assert!(infer(&prog).is_ok());
}

#[test]
fn undefined_variable_is_reported() {
    let prog = program(vec![defn("f", vec![], var("z"))]);
    let err = infer(&prog).unwrap_err();
    assert!(
        err.contains("Undefined variable"),
        "unexpected error: {err}"
    );
    assert!(err.contains("'z'"), "unexpected error: {err}");
}

#[test]
fn heterogeneous_array_literal_fails_unification() {
    let prog = program(vec![defn(
        "f",
        vec![],
        array_literal(vec![int(1), bool_lit(true)]),
    )]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn addr_of_and_deref_round_trip_to_the_same_type() {
    let prog = program(vec![defn("f", vec![("x", None)], deref(addr_of(var("x"))))]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn new_returns_the_struct_type_for_a_known_struct() {
    let prog = program(vec![
        defstruct("Point", vec![("x", "i64"), ("y", "i64")]),
        defn("f", vec![], new_expr("Point", None)),
    ]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Struct("Point".to_string()))
        }
    );
}

#[test]
fn new_returns_a_pointer_for_a_primitive_type() {
    let prog = program(vec![defn("f", vec![], new_expr("i64", Some(int(10))))]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Pointer(Box::new(Type::Int64)))
        }
    );
}

#[test]
fn new_returns_the_array_type_for_an_arr_prefixed_type_name() {
    // Regression test: `parse_type_str` used to treat any non-primitive
    // name (including koi-assembly's "arr_T" mangled-array convention) as a
    // struct name, so `(new arr_i64 ...)` would infer to
    // `Pointer<Struct("arr_i64")>` instead of `Array<Int64>`.
    let prog = program(vec![defn(
        "f",
        vec![],
        new_expr("arr_i64", Some(int(160))),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Array(Box::new(Type::Int64)))
        }
    );
}

#[test]
fn new_returns_the_array_type_for_a_nested_arr_prefixed_type_name() {
    let prog = program(vec![defn(
        "f",
        vec![],
        new_expr("arr_f64", Some(int(80))),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Array(Box::new(Type::Float64)))
        }
    );
}

#[test]
fn index_returns_the_array_element_type() {
    let prog = program(vec![defn(
        "f",
        vec![],
        index(array_literal(vec![int(1), int(2)]), int(0)),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Int64)
        }
    );
}

#[test]
fn set_var_yields_unit_type() {
    let prog = program(vec![defn(
        "f",
        vec![],
        let_binding(vec![("a", int(1))], set_var("a", int(2))),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Unit)
        }
    );
}

#[test]
fn set_var_requires_same_type_as_existing_binding() {
    let prog = program(vec![defn(
        "f",
        vec![],
        let_binding(vec![("a", int(1))], set_var("a", bool_lit(true))),
    )]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn set_var_of_undeclared_name_is_reported() {
    let prog = program(vec![defn("f", vec![], set_var("z", int(1)))]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("not declared"), "unexpected error: {err}");
    assert!(err.contains("'z'"), "unexpected error: {err}");
}

#[test]
fn while_yields_unit_type() {
    let prog = program(vec![defn(
        "f",
        vec![],
        while_expr(bool_lit(false), int(1)),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::Unit)
        }
    );
}

#[test]
fn while_requires_a_bool_condition() {
    // `n`'s condition use forces it to Bool, but the arithmetic in the body
    // forces it to Int64 -- an unresolved type var would let either
    // constraint win silently, so this needs both to pin the conflict down.
    let prog = program(vec![defn(
        "f",
        vec![("n", None)],
        while_expr(var("n"), call_named("+", vec![var("n"), int(1)])),
    )]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn do_expr_yields_the_last_expressions_type() {
    let prog = program(vec![defn(
        "f",
        vec![],
        do_expr(vec![int(1), bool_lit(true), string_lit("x")]),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![],
            return_type: Box::new(Type::String)
        }
    );
}

#[test]
fn empty_do_expr_is_rejected() {
    let prog = program(vec![defn("f", vec![], do_expr(vec![]))]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("'do'"), "unexpected error: {err}");
}

#[test]
fn aset_infers_array_element_type_and_yields_unit() {
    let prog = program(vec![defn(
        "f",
        vec![("arr", None)],
        call_named("aset!", vec![var("arr"), int(0), int(5)]),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![Type::Array(Box::new(Type::Int64))],
            return_type: Box::new(Type::Unit)
        }
    );
}

#[test]
fn aset_wrong_arg_count_is_rejected() {
    let prog = program(vec![defn(
        "f",
        vec![("arr", None)],
        call_named("aset!", vec![var("arr"), int(0)]),
    )]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("aset!"), "unexpected error: {err}");
}

#[test]
fn aset_value_type_must_match_element_type() {
    let prog = program(vec![defn(
        "f",
        vec![],
        call_named(
            "aset!",
            vec![array_literal(vec![int(1), int(2)]), int(0), bool_lit(true)],
        ),
    )]);
    let err = infer(&prog).unwrap_err();
    assert!(err.contains("Type mismatch"), "unexpected error: {err}");
}

#[test]
fn loop_returns_the_loop_variables_final_type() {
    let prog = program(vec![defn(
        "f",
        vec![("n", None)],
        loop_expr(
            "i",
            int(0),
            call_named("<", vec![var("i"), var("n")]),
            call_named("+", vec![var("i"), int(1)]),
            call_named("+", vec![var("i"), int(1)]),
        ),
    )]);
    let functions = infer(&prog).unwrap();
    assert_eq!(
        fn_type(&functions, "f"),
        Type::Function {
            params: vec![Type::Int64],
            return_type: Box::new(Type::Int64)
        }
    );
}
