//! End-to-end runs of the full pipeline (inference -> unification ->
//! monomorphization -> lambda lifting -> IR generation) on hand-built
//! programs equivalent to the project's `.carp` samples, mirroring
//! koi-ast's schema_tests.rs -- but self-contained (no dependency on
//! koi-ast's binary or its `.carp` files, since koi-ir deliberately
//! doesn't link against koi-ast).
#[path = "support.rs"]
mod support;
use support::*;

use koi::middle_end::ir::Instruction;
use koi::middle_end::pipeline;
use serde_json::Value;

/// Every JSON object produced by the IR must carry the fields the schema
/// promises: every instruction has `op`, every function has all of
/// name/returnType/parameters/blocks.
fn assert_schema(value: &Value) {
    match value {
        Value::Object(map) => {
            if map.contains_key("blocks") {
                assert!(map.contains_key("name"));
                assert!(map.contains_key("returnType"));
                assert!(map.contains_key("parameters"));
            }
            if map.contains_key("label") {
                assert!(map.contains_key("instructions"));
            }
            for v in map.values() {
                assert_schema(v);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_schema(item);
            }
        }
        _ => {}
    }
}

fn assert_no_unresolved_type_variables(value: &Value) {
    assert!(
        !contains_unresolved_type_var(value),
        "found an unresolved type variable in: {value}"
    );
}

/// Walks the JSON tree looking for a string leaf matching
/// `Type::mangled_name`'s `T{id}` format for an unbound type variable.
fn contains_unresolved_type_var(value: &Value) -> bool {
    match value {
        Value::String(s) => is_type_var_string(s),
        Value::Object(map) => map.values().any(contains_unresolved_type_var),
        Value::Array(items) => items.iter().any(contains_unresolved_type_var),
        _ => false,
    }
}

fn is_type_var_string(s: &str) -> bool {
    s.strip_prefix('T')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

#[test]
fn add_like_program_produces_schema_valid_ir_with_no_unresolved_types() {
    let prog = program(vec![
        defn(
            "add",
            vec![("x", None), ("y", None)],
            call_named("+", vec![var("x"), var("y")]),
        ),
        defn("main", vec![], call_named("add", vec![int(5), int(3)])),
    ]);

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    let value = serde_json::to_value(&ir).unwrap();
    assert_eq!(value["irType"], "hir");
    assert_schema(&value);
    assert_no_unresolved_type_variables(&value);

    let add = ir.functions.iter().find(|f| f.name == "add").unwrap();
    assert_eq!(add.return_type, "i64");
    assert_eq!(
        add.parameters,
        vec![
            ("x".to_string(), "i64".to_string()),
            ("y".to_string(), "i64".to_string())
        ]
    );
}

#[test]
fn fib_like_program_produces_real_branch_and_phi_ir() {
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

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    assert_no_unresolved_type_variables(&serde_json::to_value(&ir).unwrap());

    let fib = ir.functions.iter().find(|f| f.name == "fib").unwrap();
    assert_eq!(fib.return_type, "i64");
    assert!(
        fib.blocks.len() >= 4,
        "expected entry/then/else/merge blocks, got {} blocks",
        fib.blocks.len()
    );
    let has_phi = fib
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i, Instruction::Phi { .. }));
    assert!(has_phi, "expected a phi merging the if-branches");
    let recursive_calls = fib
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter(|i| matches!(i, Instruction::Call { function, .. } if function == "fib"))
        .count();
    assert_eq!(recursive_calls, 2);
}

#[test]
fn lambda_like_program_lifts_and_produces_call_indirect() {
    let prog = program(vec![
        defn(
            "apply-func",
            vec![("f", None), ("x", None)],
            call_named("f", vec![var("x")]),
        ),
        defn(
            "main",
            vec![],
            call_named(
                "apply-func",
                vec![
                    lambda(vec![("y", None)], call_named("+", vec![var("y"), int(1)])),
                    int(5),
                ],
            ),
        ),
    ]);

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    let value = serde_json::to_value(&ir).unwrap();
    assert_schema(&value);
    assert_no_unresolved_type_variables(&value);

    assert!(
        ir.functions.iter().any(|f| f.name.starts_with("_lambda_")),
        "expected a lifted lambda function"
    );

    let apply_func = ir
        .functions
        .iter()
        .find(|f| f.name == "apply-func")
        .unwrap();
    let has_call_indirect = apply_func
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .any(|i| matches!(i, Instruction::CallIndirect { .. }));
    assert!(
        has_call_indirect,
        "apply-func's indirect call through `f` should emit call_indirect"
    );
}

#[test]
fn struct_like_program_resolves_field_access_and_alloc() {
    let prog = program(vec![
        defstruct("Point", vec![("x", "i64"), ("y", "i64")]),
        defn("make-origin", vec![], new_expr("Point", None)),
        defn("get-x", vec![("p", None)], field_access(var("p"), "x")),
    ]);

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    assert_no_unresolved_type_variables(&serde_json::to_value(&ir).unwrap());

    let make_origin = ir
        .functions
        .iter()
        .find(|f| f.name == "make-origin")
        .unwrap();
    assert_eq!(make_origin.return_type, "Point");
    assert!(
        make_origin
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|i| matches!(i, Instruction::Alloc { ty, .. } if ty == "Point"))
    );

    let get_x = ir.functions.iter().find(|f| f.name == "get-x").unwrap();
    assert_eq!(
        get_x.parameters,
        vec![("p".to_string(), "Point".to_string())]
    );
    assert_eq!(get_x.return_type, "i64");
}

#[test]
fn control_flow_like_program_produces_a_real_iterating_loop() {
    let prog = program(vec![defn(
        "sum-below",
        vec![("n", None)],
        loop_expr(
            "i",
            int(0),
            call_named("<", vec![var("i"), var("n")]),
            call_named("+", vec![var("i"), int(1)]),
            call_named("+", vec![var("i"), int(1)]),
        ),
    )]);

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    assert_no_unresolved_type_variables(&serde_json::to_value(&ir).unwrap());

    let f = ir.functions.iter().find(|f| f.name == "sum-below").unwrap();
    let header = f
        .blocks
        .iter()
        .find(|b| b.label.starts_with("loop_header"))
        .expect("expected a loop header block");
    let phi = header.instructions.iter().find_map(|i| {
        if let Instruction::Phi { incoming, .. } = i {
            Some(incoming)
        } else {
            None
        }
    });
    assert_eq!(
        phi.map(Vec::len),
        Some(2),
        "loop header's phi should have both the entry and back edges"
    );
}

#[test]
fn array_literal_with_more_than_eight_elements_allocates_and_writes_every_element() {
    // Regression test for the heap-corruption bug: array literals used to
    // always allocate a hardcoded 64-byte buffer (`size: None` -> 8
    // elements' worth of space) no matter how many elements the literal
    // actually had. A 20-element literal exercises that boundary directly.
    // This IR-level test can't observe the actual out-of-bounds write (that
    // only happens in codegen/at runtime), but it does confirm the fix's
    // IR-level effect: `Alloc` now requests `elements.len() * 8` bytes, and
    // every element still gets its own correctly-indexed `SetIndex`.
    let n = 20;
    let elements: Vec<_> = (1..=n).map(int).collect();
    let prog = program(vec![defn("f", vec![], array_literal(elements))]);

    let ir = pipeline::compile(&prog).expect("expected the pipeline to succeed");
    assert_no_unresolved_type_variables(&serde_json::to_value(&ir).unwrap());

    let f = ir.functions.iter().find(|f| f.name == "f").unwrap();
    let instrs: Vec<&Instruction> = f.blocks.iter().flat_map(|b| &b.instructions).collect();

    // The Alloc must carry an explicit size (not the old `None` fallback),
    // and that size must resolve to exactly n * 8 bytes.
    let alloc_size_temp = instrs
        .iter()
        .find_map(|i| match i {
            Instruction::Alloc { size, .. } => Some(size.clone()),
            _ => None,
        })
        .expect("expected an Alloc instruction")
        .expect("expected Alloc.size to be Some(_), not None");
    let alloc_size_value = instrs
        .iter()
        .find_map(|i| match i {
            Instruction::Const { result, value, .. } if *result == alloc_size_temp => {
                Some(value.clone())
            }
            _ => None,
        })
        .expect("expected a Const producing the alloc size");
    assert_eq!(
        alloc_size_value,
        serde_json::json!((n * 8) as i64),
        "a {n}-element array should allocate {} bytes, not the old hardcoded 64",
        n * 8
    );

    // Every element must have been written via its own SetIndex, at the
    // right index, with the right value -- confirming the fix doesn't just
    // allocate enough space but the existing per-element write loop still
    // pairs indices/values correctly at this element count.
    let set_indices: Vec<(&String, &String)> = instrs
        .iter()
        .filter_map(|i| match i {
            Instruction::SetIndex { index, value, .. } => Some((index, value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        set_indices.len(),
        n as usize,
        "expected exactly {n} set_index instructions, one per element"
    );

    let const_value_of = |temp: &String| -> Value {
        instrs
            .iter()
            .find_map(|i| match i {
                Instruction::Const { result, value, .. } if result == temp => Some(value.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no const instruction produced temp {temp}"))
    };

    let mut pairs: Vec<(i64, i64)> = set_indices
        .into_iter()
        .map(|(index_temp, value_temp)| {
            let idx = const_value_of(index_temp).as_i64().unwrap();
            let val = const_value_of(value_temp).as_i64().unwrap();
            (idx, val)
        })
        .collect();
    pairs.sort_by_key(|(idx, _)| *idx);

    let expected: Vec<(i64, i64)> = (0..n).map(|i| (i, i + 1)).collect();
    assert_eq!(
        pairs, expected,
        "every element (including the 9th through 20th, past the old 8-element limit) \
         must be written at its correct index with its correct value"
    );
}

#[test]
fn type_error_is_reported_with_the_unification_phase_prefix() {
    let prog = program(vec![defn(
        "f",
        vec![],
        array_literal(vec![int(1), bool_lit(true)]),
    )]);
    let err = pipeline::compile(&prog).unwrap_err();
    assert!(err.starts_with("[unification]"), "unexpected error: {err}");
}

#[test]
fn undefined_variable_is_reported_with_the_inference_phase_prefix() {
    let prog = program(vec![defn("f", vec![], var("z"))]);
    let err = pipeline::compile(&prog).unwrap_err();
    assert!(err.starts_with("[inference]"), "unexpected error: {err}");
}
