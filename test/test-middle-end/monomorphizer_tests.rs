#[path = "support.rs"]
mod support;
use support::*;

use koi::frontend::ast::ASTNode;
use koi::middle_end::monomorphizer::Monomorphizer;
use koi::middle_end::types::Type;
use std::collections::HashMap;

fn function_names(node: &ASTNode) -> Vec<String> {
    match node {
        ASTNode::Program { children } => children
            .iter()
            .filter_map(|c| {
                if let ASTNode::FunctionDef { name, .. } = c {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect(),
        _ => vec![],
    }
}

#[test]
fn mangle_name_joins_type_names_with_double_underscore() {
    assert_eq!(Monomorphizer::mangle_name("id", &[Type::Int64]), "id__i64");
    assert_eq!(
        Monomorphizer::mangle_name("id", &[Type::Int64, Type::Float64]),
        "id__i64_f64"
    );
    assert_eq!(Monomorphizer::mangle_name("f", &[]), "f__");
}

#[test]
fn a_single_recorded_instantiation_needs_no_specialization() {
    let mut mono = Monomorphizer::new();
    mono.record_call("id", vec![Type::Int64]);
    assert!(mono.specializations_needed().is_empty());
}

#[test]
fn recording_the_same_tuple_twice_still_counts_as_one() {
    let mut mono = Monomorphizer::new();
    mono.record_call("id", vec![Type::Int64]);
    mono.record_call("id", vec![Type::Int64]);
    assert!(mono.specializations_needed().is_empty());
}

#[test]
fn two_distinct_tuples_are_flagged_for_specialization() {
    let mut mono = Monomorphizer::new();
    mono.record_call("id", vec![Type::Int64]);
    mono.record_call("id", vec![Type::Float64]);

    let needed = mono.specializations_needed();
    let tuples = needed.get("id").expect("id should need specialization");
    assert_eq!(tuples.len(), 2);
    assert!(tuples.contains(&vec![Type::Int64]));
    assert!(tuples.contains(&vec![Type::Float64]));
}

#[test]
fn collect_from_functions_records_exactly_one_instantiation_per_function() {
    // Documents the architectural invariant: every function reaching this
    // stage in the real pipeline was already unified against exactly one
    // signature, so collecting straight from `functions` can never surface
    // a function needing specialization.
    let mut functions = HashMap::new();
    functions.insert(
        "add".to_string(),
        Type::Function {
            params: vec![Type::Int64, Type::Int64],
            return_type: Box::new(Type::Int64),
        },
    );
    functions.insert(
        "id".to_string(),
        Type::Function {
            params: vec![Type::Bool],
            return_type: Box::new(Type::Bool),
        },
    );

    let mut mono = Monomorphizer::new();
    mono.collect_from_functions(&functions);

    assert!(mono.specializations_needed().is_empty());
}

#[test]
fn specialize_program_is_a_no_op_when_nothing_needs_specializing() {
    let prog = program(vec![defn("f", vec![], int(1))]);
    let mono = Monomorphizer::new();
    let result = mono.specialize_program(&prog);
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        serde_json::to_string(&prog).unwrap()
    );
}

#[test]
fn specialize_program_clones_and_mangles_a_function_with_two_instantiations() {
    let prog = program(vec![defn("id", vec![("x", None)], var("x"))]);

    let mut mono = Monomorphizer::new();
    mono.record_call("id", vec![Type::Int64]);
    mono.record_call("id", vec![Type::Float64]);

    let result = mono.specialize_program(&prog);
    let mut names = function_names(&result);
    names.sort();
    assert_eq!(names, vec!["id__f64".to_string(), "id__i64".to_string()]);
}

#[test]
fn specialize_program_leaves_unrelated_functions_untouched() {
    let prog = program(vec![
        defn("id", vec![("x", None)], var("x")),
        defn("other", vec![], int(42)),
    ]);

    let mut mono = Monomorphizer::new();
    mono.record_call("id", vec![Type::Int64]);
    mono.record_call("id", vec![Type::Float64]);

    let result = mono.specialize_program(&prog);
    let names = function_names(&result);
    assert!(names.contains(&"other".to_string()));
    assert!(
        !names.contains(&"id".to_string()),
        "unspecialized 'id' should not survive"
    );
}
