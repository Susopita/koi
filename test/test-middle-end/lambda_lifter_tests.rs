#[path = "support.rs"]
mod support;
use support::*;

use koi::frontend::ast::ASTNode;
use koi::middle_end::lambda_lifter::LambdaLifter;

type FunctionDefView<'a> = (&'a str, &'a [(String, Option<String>)], &'a ASTNode);

fn function_defs(node: &ASTNode) -> Vec<FunctionDefView<'_>> {
    match node {
        ASTNode::Program { children } => children
            .iter()
            .filter_map(|c| match c {
                ASTNode::FunctionDef {
                    name,
                    parameters,
                    body,
                    ..
                } => Some((name.as_str(), parameters.as_slice(), body.as_ref())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

fn struct_defs(node: &ASTNode) -> Vec<(&str, &[(String, String)])> {
    match node {
        ASTNode::Program { children } => children
            .iter()
            .filter_map(|c| match c {
                ASTNode::StructDef { name, fields, .. } => Some((name.as_str(), fields.as_slice())),
                _ => None,
            })
            .collect(),
        _ => vec![],
    }
}

#[test]
fn zero_capture_lambda_lifts_to_a_top_level_function() {
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

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    let lifted_fn = defs
        .iter()
        .find(|(name, ..)| name.starts_with("_lambda_"))
        .expect("expected a lifted lambda function");
    assert_eq!(
        lifted_fn.1.len(),
        1,
        "zero-capture lambda should keep just its own parameter"
    );
    assert_eq!(lifted_fn.1[0].0, "y");

    // The call site's lambda argument becomes a bare reference to the
    // lifted function's name.
    let main_body = defs.iter().find(|(name, ..)| *name == "main").unwrap().2;
    let ASTNode::Call { arguments, .. } = main_body else {
        panic!("expected a call")
    };
    assert!(matches!(&arguments[0], ASTNode::Variable { name, .. } if name == lifted_fn.0));
}

#[test]
fn builtins_and_sibling_functions_are_never_treated_as_captures() {
    // Regression: without excluding builtins/globals, a lambda referencing
    // `+` or a sibling top-level function would look like it captured them.
    let prog = program(vec![
        defn("helper", vec![], int(1)),
        defn(
            "main",
            vec![],
            lambda(
                vec![("y", None)],
                call_named("+", vec![var("y"), call_named("helper", vec![])]),
            ),
        ),
    ]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    let lifted_fn = defs
        .iter()
        .find(|(name, ..)| name.starts_with("_lambda_"))
        .expect("expected a lifted lambda function");
    assert_eq!(
        lifted_fn.1.len(),
        1,
        "referencing builtins/globals must not add an env capture"
    );
}

#[test]
fn a_captured_free_variable_produces_an_env_struct_and_rewrites_access() {
    let prog = program(vec![defn(
        "outer",
        vec![("x", None)],
        lambda(vec![("y", None)], call_named("+", vec![var("y"), var("x")])),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let structs = struct_defs(&lifted);
    let env_struct = structs
        .first()
        .expect("expected an env struct to be generated");
    assert_eq!(
        env_struct
            .1
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["x"]
    );

    let defs = function_defs(&lifted);
    let lifted_fn = defs
        .iter()
        .find(|(name, ..)| name.starts_with("_lambda_"))
        .unwrap();
    assert_eq!(
        lifted_fn.1[0].0, "env",
        "env must be the first parameter of a lifted closure"
    );

    // `x` inside the lifted body must have become `(field env x)`.
    let body_json = serde_json::to_string(lifted_fn.2).unwrap();
    assert!(
        body_json.contains("\"field\":\"x\""),
        "expected a rewritten field access to x, got: {body_json}"
    );
    assert!(
        body_json.contains("\"name\":\"env\""),
        "expected the field access object to be `env`, got: {body_json}"
    );
}

#[test]
fn a_let_binding_inside_the_lambda_shadows_a_would_be_capture() {
    // The lambda's own `let` re-binds `x`, so the reference to `x` in the
    // let's body refers to the local binding, not anything from an
    // enclosing scope -- it must not be captured.
    let prog = program(vec![defn(
        "outer",
        vec![("x", None)],
        lambda(
            vec![("y", None)],
            let_binding(
                vec![("x", int(99))],
                call_named("+", vec![var("y"), var("x")]),
            ),
        ),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    let lifted_fn = defs
        .iter()
        .find(|(name, ..)| name.starts_with("_lambda_"))
        .unwrap();
    assert_eq!(
        lifted_fn.1.len(),
        1,
        "the let-shadowed x must not become a capture"
    );
}

#[test]
fn nested_lambda_capture_does_not_leak_into_the_outer_lambdas_own_analysis() {
    // Regression: the outer lambda's free-variable analysis runs on the
    // *already-lifted* inner lambda's replacement node. If the lifted
    // inner function's name (or its closure-constructor placeholder) isn't
    // recognized as global, the outer lambda would wrongly think it needs
    // to capture that made-up name too.
    let prog = program(vec![defn(
        "use-it",
        vec![],
        lambda(
            vec![("x", None)],
            lambda(vec![("y", None)], call_named("+", vec![var("x"), var("y")])),
        ),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    // The inner lambda captures `x`, so exactly one lifted function should
    // have an `env` parameter; the outer lambda must not.
    let with_env: Vec<_> = defs
        .iter()
        .filter(|(_, params, _)| params.first().map(|(n, _)| n.as_str()) == Some("env"))
        .collect();
    assert_eq!(
        with_env.len(),
        1,
        "exactly the inner lambda should have captured something, got: {defs:?}"
    );

    let without_env: Vec<_> = defs
        .iter()
        .filter(|(name, params, _)| {
            name.starts_with("_lambda_") && params.first().map(|(n, _)| n.as_str()) != Some("env")
        })
        .collect();
    assert_eq!(
        without_env.len(),
        1,
        "the outer lambda must not have captured anything, got: {defs:?}"
    );
    assert_eq!(
        without_env[0].1.len(),
        1,
        "outer lambda should keep only its own parameter x"
    );
}

#[test]
fn a_lambda_parameter_shadows_an_outer_capture_of_the_same_name() {
    // The inner lambda's own parameter is also named `x`, so it must bind
    // to the *inner* parameter, not capture the outer `x`.
    let prog = program(vec![defn(
        "use-it",
        vec![],
        lambda(
            vec![("x", None)],
            lambda(vec![("x", None)], call_named("+", vec![var("x"), int(1)])),
        ),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    let with_env = defs
        .iter()
        .any(|(_, params, _)| params.first().map(|(n, _)| n.as_str()) == Some("env"));
    assert!(
        !with_env,
        "shadowed parameter name must not be captured, got: {defs:?}"
    );
}

#[test]
fn a_function_using_while_do_and_set_lifts_without_crashing() {
    // Free-variable analysis and node reconstruction must handle the new
    // SetVar/WhileExpr/DoExpr node types without panicking, even though
    // deep closure-capture-of-a-mutated-variable behavior is out of scope.
    let prog = program(vec![defn(
        "count-to",
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

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let defs = function_defs(&lifted);
    let count_to = defs
        .iter()
        .find(|(name, ..)| *name == "count-to")
        .expect("expected count-to to survive lifting");
    // Sanity check the shape survived: still a let-binding wrapping a do
    // wrapping a while and a trailing variable reference.
    let ASTNode::LetBinding { body, .. } = count_to.2 else {
        panic!("expected a let-binding body, got: {:?}", count_to.2)
    };
    assert!(matches!(body.as_ref(), ASTNode::DoExpr { .. }));
}

#[test]
fn a_lambda_capturing_a_variable_used_in_while_and_set_is_still_captured() {
    // The lambda's body uses `x` (from the enclosing function) inside a
    // `while`/`set!`; free-variable analysis must still see `x` as a
    // capture even though it's inside these new node types.
    let prog = program(vec![defn(
        "outer",
        vec![("x", None)],
        lambda(
            vec![("y", None)],
            do_expr(vec![
                while_expr(
                    call_named("<", vec![var("y"), var("x")]),
                    set_var("y", call_named("+", vec![var("y"), int(1)])),
                ),
                var("y"),
            ]),
        ),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let structs = struct_defs(&lifted);
    let env_struct = structs
        .first()
        .expect("expected an env struct capturing 'x'");
    assert_eq!(
        env_struct
            .1
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["x"]
    );
}

#[test]
fn a_captured_free_variable_lifts_to_a_make_closure_node_not_a_placeholder_call() {
    // Regression: capturing lambdas used to lower to a placeholder `Call`
    // targeting a made-up `__make_closure_{func_name}` function name that
    // nothing anywhere ever defined -- which is exactly why koi-assembly
    // failed with "no home allocated for value '__make_closure__lambda_0'"
    // whenever a lambda captured a free variable. They must lower to a
    // dedicated `MakeClosure` node instead, carrying the lifted function's
    // name and the exact set of captured variable names; `ir_generator.rs`
    // is what actually constructs the closure value from this node, using
    // real (post-monomorphization) types unavailable at this stage.
    let prog = program(vec![defn(
        "outer",
        vec![("x", None)],
        lambda(vec![("y", None)], call_named("+", vec![var("y"), var("x")])),
    )]);

    let mut lifter = LambdaLifter::for_program(&prog);
    let lifted = lifter.lift_program(&prog);

    let outer = function_defs(&lifted)
        .into_iter()
        .find(|(name, ..)| *name == "outer")
        .expect("expected 'outer' to survive lifting");

    let ASTNode::MakeClosure {
        function_name,
        captured,
        ..
    } = outer.2
    else {
        panic!(
            "expected 'outer' body to become a MakeClosure node, got: {:?}",
            outer.2
        )
    };

    assert!(
        function_name.starts_with("_lambda_"),
        "expected the MakeClosure to name a lifted lambda function, got: {function_name}"
    );
    assert_eq!(captured, &vec!["x".to_string()]);

    // No placeholder `__make_closure_*` call survives anywhere in the
    // lifted program.
    let lifted_json = serde_json::to_string(&lifted).unwrap();
    assert!(
        !lifted_json.contains("__make_closure_"),
        "no placeholder __make_closure_* call should remain, got: {lifted_json}"
    );
}
