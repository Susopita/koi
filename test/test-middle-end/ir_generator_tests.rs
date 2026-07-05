#[path = "support.rs"]
mod support;
use support::*;

use koi::middle_end::ir::{BasicBlock, IRFunction, IRProgram, Instruction};
use koi::middle_end::ir_generator::IRGenerator;
use koi::middle_end::types::Type;
use std::collections::{HashMap, HashSet};

fn generate(
    prog: &koi::frontend::ast::ASTNode,
    functions: &HashMap<String, Type>,
    struct_fields: &HashMap<String, Vec<(String, Type)>>,
) -> IRProgram {
    IRGenerator::new(functions, struct_fields)
        .generate_program(prog)
        .expect("expected IR generation to succeed")
}

fn find_function<'a>(ir: &'a IRProgram, name: &str) -> &'a IRFunction {
    ir.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function named '{name}' in {ir:?}"))
}

fn all_instructions(func: &IRFunction) -> Vec<&Instruction> {
    func.blocks
        .iter()
        .flat_map(|b: &BasicBlock| b.instructions.iter())
        .collect()
}

fn count(instrs: &[&Instruction], pred: impl Fn(&Instruction) -> bool) -> usize {
    instrs.iter().filter(|i| pred(i)).count()
}

fn fn_type(params: Vec<Type>, ret: Type) -> Type {
    Type::Function {
        params,
        return_type: Box::new(ret),
    }
}

#[test]
fn literal_emits_a_const_and_the_function_returns_it() {
    let prog = program(vec![defn("f", vec![], int(42))]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let f = find_function(&ir, "f");
    let instrs = all_instructions(f);
    assert!(
        matches!(instrs[0], Instruction::Const { value, ty, .. } if *value == serde_json::json!(42) && ty == "i64")
    );
    assert!(matches!(
        instrs.last().unwrap(),
        Instruction::Return { value: Some(_) }
    ));
}

#[test]
fn multi_arg_arithmetic_folds_into_a_chain_of_binops() {
    let prog = program(vec![defn(
        "f",
        vec![],
        call_named("+", vec![int(1), int(2), int(3)]),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert_eq!(
        count(
            &instrs,
            |i| matches!(i, Instruction::BinOp { op_type, .. } if op_type == "+")
        ),
        2
    );
}

#[test]
fn comparison_emits_a_bool_typed_binop() {
    let prog = program(vec![defn(
        "f",
        vec![],
        call_named("<", vec![int(1), int(2)]),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Bool))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(instrs.iter().any(
        |i| matches!(i, Instruction::BinOp { op_type, ty, .. } if op_type == "<" && ty == "bool")
    ));
}

#[test]
fn logical_not_emits_an_equals_false_binop() {
    let prog = program(vec![defn(
        "f",
        vec![],
        call_named("!", vec![bool_lit(true)]),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Bool))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(instrs.iter().any(
        |i| matches!(i, Instruction::Const { value, .. } if *value == serde_json::json!(false))
    ));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::BinOp { op_type, .. } if op_type == "=="))
    );
}

#[test]
fn calling_a_known_top_level_function_emits_call() {
    let prog = program(vec![defn(
        "main",
        vec![],
        call_named("add", vec![int(1), int(2)]),
    )]);
    let functions = HashMap::from([
        ("main".to_string(), fn_type(vec![], Type::Int64)),
        (
            "add".to_string(),
            fn_type(vec![Type::Int64, Type::Int64], Type::Int64),
        ),
    ]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "main"));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::Call { function, .. } if function == "add"))
    );
    assert!(
        !instrs
            .iter()
            .any(|i| matches!(i, Instruction::CallIndirect { .. }))
    );
}

#[test]
fn calling_a_function_valued_parameter_emits_call_indirect() {
    let prog = program(vec![defn(
        "apply-func",
        vec![("f", None), ("x", None)],
        call_named("f", vec![var("x")]),
    )]);
    let functions = HashMap::from([(
        "apply-func".to_string(),
        fn_type(
            vec![fn_type(vec![Type::Int64], Type::Int64), Type::Int64],
            Type::Int64,
        ),
    )]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "apply-func"));
    assert!(instrs.iter().any(
        |i| matches!(i, Instruction::CallIndirect { function_value, .. } if function_value == "f")
    ));
    assert!(!instrs.iter().any(|i| matches!(i, Instruction::Call { .. })));
}

#[test]
fn if_expression_branches_and_merges_with_a_phi() {
    let prog = program(vec![defn(
        "f",
        vec![("x", None)],
        if_expr(var("x"), int(1), Some(int(2))),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![Type::Bool], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let f = find_function(&ir, "f");
    assert_eq!(
        f.blocks.len(),
        4,
        "expected entry/then/else/merge blocks, got: {f:?}"
    );

    let entry = &f.blocks[0];
    assert!(matches!(
        entry.instructions.last().unwrap(),
        Instruction::Branch { .. }
    ));

    let merge = f.blocks.last().unwrap();
    let phi = merge
        .instructions
        .iter()
        .find(|i| matches!(i, Instruction::Phi { .. }))
        .expect("expected a phi in the merge block");
    let Instruction::Phi { incoming, .. } = phi else {
        unreachable!()
    };
    assert_eq!(incoming.len(), 2);
    assert!(matches!(
        merge.instructions.last().unwrap(),
        Instruction::Return { .. }
    ));
}

#[test]
fn loop_header_phi_gets_its_back_edge_patched() {
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
    let functions = HashMap::from([("f".to_string(), fn_type(vec![Type::Int64], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let f = find_function(&ir, "f");
    let header = f
        .blocks
        .iter()
        .find(|b| b.label.starts_with("loop_header"))
        .expect("expected a loop header block");
    let phi = header
        .instructions
        .iter()
        .find(|i| matches!(i, Instruction::Phi { .. }))
        .expect("expected a phi in the loop header");
    let Instruction::Phi { incoming, .. } = phi else {
        unreachable!()
    };
    assert_eq!(
        incoming.len(),
        2,
        "expected both the pre-loop edge and the patched back-edge"
    );

    let body = f
        .blocks
        .iter()
        .find(|b| b.label.starts_with("loop_body"))
        .expect("expected a loop body block");
    assert!(
        matches!(body.instructions.last().unwrap(), Instruction::Jump { label } if label.starts_with("loop_header"))
    );
}

#[test]
fn field_access_emits_get_field() {
    let prog = program(vec![defn(
        "get-x",
        vec![("p", None)],
        field_access(var("p"), "x"),
    )]);
    let functions = HashMap::from([(
        "get-x".to_string(),
        fn_type(vec![Type::Struct("Point".to_string())], Type::Int64),
    )]);
    let struct_fields = HashMap::from([(
        "Point".to_string(),
        vec![
            ("x".to_string(), Type::Int64),
            ("y".to_string(), Type::Int64),
        ],
    )]);
    let ir = generate(&prog, &functions, &struct_fields);

    let instrs = all_instructions(find_function(&ir, "get-x"));
    assert!(instrs.iter().any(
        |i| matches!(i, Instruction::GetField { field, ty, .. } if field == "x" && ty == "i64")
    ));
}

#[test]
fn set_field_emits_set_field_with_the_real_field_type_and_yields_unit() {
    let prog = program(vec![defn(
        "set-x",
        vec![("p", None), ("v", None)],
        set_field(var("p"), "x", var("v")),
    )]);
    let functions = HashMap::from([(
        "set-x".to_string(),
        fn_type(
            vec![Type::Struct("Point".to_string()), Type::Int64],
            Type::Unit,
        ),
    )]);
    let struct_fields = HashMap::from([(
        "Point".to_string(),
        vec![
            ("x".to_string(), Type::Int64),
            ("y".to_string(), Type::Int64),
        ],
    )]);
    let ir = generate(&prog, &functions, &struct_fields);

    let instrs = all_instructions(find_function(&ir, "set-x"));
    assert!(instrs.iter().any(
        |i| matches!(i, Instruction::SetField { field, ty, .. } if field == "x" && ty == "i64")
    ));
    // No result-bearing use of the SetField itself -- its "value" is a
    // synthesized unit, same convention as `set!`/`while`/`aset!`.
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::Const { ty, value, .. } if ty == "unit" && value.is_null()))
    );
}

#[test]
fn index_emits_get_index() {
    let prog = program(vec![defn(
        "f",
        vec![],
        index(array_literal(vec![int(1), int(2)]), int(0)),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::Alloc { .. }))
    );
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::GetIndex { .. }))
    );
}

#[test]
fn array_literal_allocates_exactly_enough_space_for_all_elements() {
    // Regression test: array literals used to always pass `size: None` to
    // `Alloc`, which fell back to a hardcoded 64-byte buffer in codegen
    // regardless of element count -- any literal with more than 8 elements
    // (8 * 8 bytes = 64) silently wrote past the allocated block. A 20-int64
    // literal needs 160 bytes; if this regresses back to `size: None` (or to
    // a wrong byte count), this test should catch it.
    let elements: Vec<_> = (1..=20).map(int).collect();
    let prog = program(vec![defn("f", vec![], array_literal(elements))]);
    let functions = HashMap::from([(
        "f".to_string(),
        fn_type(vec![], Type::Array(Box::new(Type::Int64))),
    )]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    let alloc_size = instrs
        .iter()
        .find_map(|i| match i {
            Instruction::Alloc { size, .. } => Some(size.clone()),
            _ => None,
        })
        .expect("expected an Alloc instruction");
    let size_temp = alloc_size.expect("expected Alloc.size to be Some(_), not None");

    let size_value = instrs
        .iter()
        .find_map(|i| match i {
            Instruction::Const { result, value, .. } if *result == size_temp => {
                Some(value.clone())
            }
            _ => None,
        })
        .expect("expected a Const instruction producing the size temp");
    assert_eq!(size_value, serde_json::json!(160), "20 elements * 8 bytes");
}

#[test]
fn addr_of_and_deref_emit_dedicated_instructions() {
    let prog = program(vec![defn("f", vec![("x", None)], deref(addr_of(var("x"))))]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![Type::Int64], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::AddrOf { .. }))
    );
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::Deref { .. }))
    );
}

#[test]
fn new_emits_alloc() {
    let prog = program(vec![defn("f", vec![], new_expr("Point", None))]);
    let functions = HashMap::from([(
        "f".to_string(),
        fn_type(vec![], Type::Struct("Point".to_string())),
    )]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::Alloc { ty, .. } if ty == "Point"))
    );
}

#[test]
fn every_function_ends_in_a_return() {
    let prog = program(vec![
        defn(
            "f",
            vec![("x", None)],
            if_expr(var("x"), int(1), Some(int(2))),
        ),
        defn("g", vec![], int(1)),
    ]);
    let functions = HashMap::from([
        ("f".to_string(), fn_type(vec![Type::Bool], Type::Int64)),
        ("g".to_string(), fn_type(vec![], Type::Int64)),
    ]);
    let ir = generate(&prog, &functions, &HashMap::new());

    for f in &ir.functions {
        let last_block = f.blocks.last().unwrap();
        assert!(
            matches!(
                last_block.instructions.last().unwrap(),
                Instruction::Return { .. }
            ),
            "function {} doesn't end in return",
            f.name
        );
    }
}

#[test]
fn ssa_results_are_never_assigned_twice() {
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
    let functions = HashMap::from([("fib".to_string(), fn_type(vec![Type::Int64], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "fib"));
    let results: Vec<&str> = instrs
        .iter()
        .filter_map(|i| match i {
            Instruction::Const { result, .. }
            | Instruction::BinOp { result, .. }
            | Instruction::Phi { result, .. }
            | Instruction::Alloc { result, .. }
            | Instruction::GetField { result, .. }
            | Instruction::GetIndex { result, .. }
            | Instruction::AddrOf { result, .. }
            | Instruction::Deref { result, .. } => Some(result.as_str()),
            Instruction::Call {
                result: Some(r), ..
            }
            | Instruction::CallIndirect {
                result: Some(r), ..
            } => Some(r.as_str()),
            _ => None,
        })
        .collect();

    let unique: HashSet<&str> = results.iter().copied().collect();
    assert_eq!(
        results.len(),
        unique.len(),
        "an SSA temp was assigned more than once: {results:?}"
    );
}

#[test]
fn while_with_set_lowers_to_a_phi_with_two_incoming_edges_and_a_branch() {
    // `(defn f (n) (let ((i 0)) (do (while (< i n) (set! i (+ i 1))) i)))`
    let prog = program(vec![defn(
        "f",
        vec![("n", None)],
        let_binding(
            vec![("i", int(0))],
            do_expr(vec![
                while_expr(
                    call_named("<", vec![var("i"), var("n")]),
                    set_var("i", call_named("+", vec![var("i"), int(1)])),
                ),
                var("i"),
            ]),
        ),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![Type::Int64], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let f = find_function(&ir, "f");
    let header = f
        .blocks
        .iter()
        .find(|b| b.label.starts_with("while_header"))
        .expect("expected a while header block");
    let phi = header
        .instructions
        .iter()
        .find(|i| matches!(i, Instruction::Phi { .. }))
        .expect("expected a phi in the while header for the mutated variable 'i'");
    let Instruction::Phi { incoming, .. } = phi else {
        unreachable!()
    };
    assert_eq!(
        incoming.len(),
        2,
        "expected both the pre-loop edge and the patched back-edge"
    );

    assert!(
        header
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Branch { .. })),
        "expected the while header to end in a branch on the condition"
    );

    let body = f
        .blocks
        .iter()
        .find(|b| b.label.starts_with("while_body"))
        .expect("expected a while body block");
    assert!(
        matches!(body.instructions.last().unwrap(), Instruction::Jump { label } if label.starts_with("while_header")),
        "expected the while body's latch block to jump back to the header"
    );
}

#[test]
fn do_expression_propagates_only_the_last_exprs_value() {
    let prog = program(vec![defn(
        "f",
        vec![],
        do_expr(vec![int(1), int(2), int(3)]),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let f = find_function(&ir, "f");
    let instrs = all_instructions(f);
    // All three literals are still evaluated (in order)...
    assert_eq!(
        count(
            &instrs,
            |i| matches!(i, Instruction::Const { ty, .. } if ty == "i64")
        ),
        3
    );
    // ...but the function returns the temp produced by the *last* one (3),
    // not the first (1) or second (2).
    let last_const_result = instrs
        .iter()
        .rev()
        .find_map(|i| match i {
            Instruction::Const { result, value, .. } if *value == serde_json::json!(3) => {
                Some(result.clone())
            }
            _ => None,
        })
        .expect("expected a const producing 3");
    assert!(matches!(
        instrs.last().unwrap(),
        Instruction::Return { value: Some(v) } if *v == last_const_result
    ));
}

#[test]
fn make_closure_allocates_an_env_struct_with_the_captured_variables_real_type() {
    // Regression setup for: `(let [factor 3] ((lambda [x] (* x factor)) 5))`
    // -- captures used to be lowered to a placeholder `Call` to a
    // `__make_closure_*` name nothing defined, which crashed koi-assembly.
    // This builds the AST the way `lambda_lifter.rs` would have produced it
    // (a `MakeClosure` node alongside a separately-lifted `_lambda_0`
    // function reading `env.factor`) and checks `ir_generator.rs` actually
    // constructs the closure -- using `factor`'s *real* type (f64 here, not
    // the lifter-era hardcoded i64 fallback) for the env struct's field.
    let prog = program(vec![
        defn(
            "main",
            vec![],
            let_binding(
                vec![("factor", float(2.5)), ("c", make_closure("_lambda_0", vec!["factor"]))],
                call_named("c", vec![int(5)]),
            ),
        ),
        defn(
            "_lambda_0",
            vec![("env", Some("env__lambda_0")), ("x", None)],
            field_access(var("env"), "factor"),
        ),
    ]);
    let functions = HashMap::from([
        ("main".to_string(), fn_type(vec![], Type::Float64)),
        (
            "_lambda_0".to_string(),
            fn_type(
                vec![Type::Struct("env__lambda_0".to_string()), Type::Int64],
                Type::Float64,
            ),
        ),
    ]);
    let ir = generate(&prog, &functions, &HashMap::new());

    // The env struct is populated via `SetField` with `factor`'s real type
    // (f64), not a hardcoded i64.
    let main_instrs = all_instructions(find_function(&ir, "main"));
    let factor_set_field = main_instrs
        .iter()
        .find_map(|i| match i {
            Instruction::SetField { field, ty, .. } if field == "factor" => Some(ty.as_str()),
            _ => None,
        })
        .expect("expected a set_field storing 'factor' into the env struct");
    assert_eq!(
        factor_set_field, "f64",
        "the env struct's captured field must carry factor's real type, not a hardcoded default"
    );

    // The shared `Closure` wrapper's own two fields (`fn_ptr`/`env_ptr`) are
    // also populated via `SetField`.
    assert!(main_instrs.iter().any(
        |i| matches!(i, Instruction::SetField { field, .. } if field == "fn_ptr")
    ));
    assert!(main_instrs.iter().any(
        |i| matches!(i, Instruction::SetField { field, .. } if field == "env_ptr")
    ));

    // The call through `c` unpacks the closure (two GetFields) and calls
    // indirectly through the function pointer with the env prepended --
    // not a raw call straight through the closure value.
    let get_fields: Vec<&str> = main_instrs
        .iter()
        .filter_map(|i| match i {
            Instruction::GetField { field, .. } => Some(field.as_str()),
            _ => None,
        })
        .collect();
    assert!(get_fields.contains(&"fn_ptr"));
    assert!(get_fields.contains(&"env_ptr"));

    let call_indirect_args = main_instrs
        .iter()
        .find_map(|i| match i {
            Instruction::CallIndirect { arguments, .. } => Some(arguments),
            _ => None,
        })
        .expect("expected a call_indirect for the closure call");
    assert_eq!(
        call_indirect_args.len(),
        2,
        "expected the env pointer prepended to the original argument list, got: {call_indirect_args:?}"
    );
    assert!(
        !main_instrs
            .iter()
            .any(|i| matches!(i, Instruction::Call { .. })),
        "a closure call must never lower to a plain (direct) Call"
    );

    // Inside `_lambda_0`, `env.factor` resolves to the real type (f64) via
    // `closure_env_types`, not the generic i64 default `field_type` used to
    // fall back to for an unrecognized struct.
    let lambda_instrs = all_instructions(find_function(&ir, "_lambda_0"));
    assert!(
        lambda_instrs.iter().any(
            |i| matches!(i, Instruction::GetField { field, ty, .. } if field == "factor" && ty == "f64")
        ),
        "expected env.factor to resolve to f64 inside the lifted lambda, got: {lambda_instrs:?}"
    );
}

#[test]
fn aset_builtin_lowers_to_a_set_index_instruction() {
    let prog = program(vec![defn(
        "f",
        vec![],
        call_named(
            "aset!",
            vec![array_literal(vec![int(1), int(2)]), int(0), int(9)],
        ),
    )]);
    let functions = HashMap::from([("f".to_string(), fn_type(vec![], Type::Int64))]);
    let ir = generate(&prog, &functions, &HashMap::new());

    let instrs = all_instructions(find_function(&ir, "f"));
    assert!(
        instrs
            .iter()
            .any(|i| matches!(i, Instruction::SetIndex { .. })),
        "expected aset! to lower to a set_index instruction, got: {instrs:?}"
    );
}
