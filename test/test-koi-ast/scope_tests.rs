use koi_ast::parser::Parser;
use koi_ast::scanner::Scanner;
use koi_ast::scope::ScopeAnalyzer;

fn analyze(src: &str) -> Result<(), Vec<String>> {
    let ast = Parser::new(Scanner::new(src))
        .parse_program()
        .unwrap_or_else(|e| panic!("expected `{src}` to parse, got error: {e}"));
    ScopeAnalyzer::new().analyze(&ast)
}

fn assert_ok(src: &str) {
    if let Err(errors) = analyze(src) {
        panic!("expected `{src}` to scope-check cleanly, got errors: {errors:?}");
    }
}

fn assert_undeclared(src: &str, var_name: &str) -> Vec<String> {
    match analyze(src) {
        Ok(()) => panic!(
            "expected `{src}` to report an undeclared variable, but it scope-checked cleanly"
        ),
        Err(errors) => {
            assert!(
                errors.iter().any(|e| e.contains(&format!("'{var_name}'"))),
                "expected an error mentioning '{var_name}', got: {errors:?}"
            );
            errors
        }
    }
}

#[test]
fn declared_parameters_are_visible_in_body() {
    assert_ok("(defn add [x y] (+ x y))");
}

#[test]
fn undeclared_variable_is_reported() {
    assert_undeclared("(defn f [x] (+ x z))", "z");
}

#[test]
fn undeclared_error_includes_location() {
    let errors = assert_undeclared("(defn f [x] (+ x z))", "z");
    assert!(
        errors[0].contains("line 1"),
        "expected location info, got: {:?}",
        errors[0]
    );
}

#[test]
fn builtin_operators_do_not_need_declaration() {
    assert_ok("(defn f [] (+ 1 (* 2 3)))");
    assert_ok("(defn f [] (- 1 (/ 4 2)))");
    assert_ok("(defn f [a b] (&& (< a b) (|| (== a b) (!= a b))))");
    assert_ok("(defn f [x] (! x))");
    assert_ok("(defn f [x] (print x))");
}

#[test]
fn forward_reference_between_top_level_functions_is_allowed() {
    // `a` calls `b`, which is only defined afterwards -- requires
    // pre-registering all top-level defn names before walking bodies.
    assert_ok("(defn a [] (b)) (defn b [] 1)");
}

#[test]
fn self_recursive_function_is_allowed() {
    assert_ok("(defn fib [n] (fib n))");
}

#[test]
fn mutually_recursive_functions_are_allowed() {
    assert_ok("(defn even? [n] (odd? n)) (defn odd? [n] (even? n))");
}

#[test]
fn let_binding_is_visible_in_body() {
    assert_ok("(defn f [] (let [a 1] a))");
}

#[test]
fn let_binding_does_not_leak_into_a_different_function() {
    assert_undeclared("(defn f [] (let [a 1] a)) (defn g [] a)", "a");
}

#[test]
fn let_bindings_see_earlier_bindings_in_the_same_let() {
    assert_ok("(defn f [] (let [a 1 b (+ a 1)] b))");
}

#[test]
fn let_body_cannot_see_a_binding_declared_after_the_body_position() {
    // Bindings are only visible from the point they're declared onward, and
    // only within the let; referencing one outside entirely is undeclared.
    assert_undeclared("(defn f [] (+ (let [a 1] a) a))", "a");
}

#[test]
fn lambda_parameter_is_visible_in_lambda_body() {
    assert_ok("(defn f [] (lambda [y] (+ y 1)))");
}

#[test]
fn lambda_parameter_does_not_leak_outside_lambda() {
    assert_undeclared("(defn f [] (let [z (lambda [y] y)] y))", "y");
}

#[test]
fn loop_variable_is_visible_in_condition_step_and_body() {
    assert_ok("(defn f [n] (loop [i 0] (< i n) (+ i 1) i))");
}

#[test]
fn loop_variable_does_not_leak_outside_loop() {
    assert_undeclared("(defn f [n] (+ (loop [i 0] (< i n) (+ i 1) i) i))", "i");
}

#[test]
fn struct_field_names_are_not_scope_checked() {
    // `x` here is a field name, not a variable -- it must not be required
    // to be a declared identifier.
    assert_ok("(defstruct Point [x i64] [y i64]) (defn f [p] (field p x))");
}

#[test]
fn new_type_name_is_not_scope_checked() {
    assert_ok("(defstruct Point [x i64] [y i64]) (defn f [] (new Point))");
}

#[test]
fn multiple_undeclared_variables_are_all_reported() {
    match analyze("(defn f [] (+ a b))") {
        Ok(()) => panic!("expected undeclared-variable errors"),
        Err(errors) => {
            assert_eq!(errors.len(), 2, "expected two errors, got: {errors:?}");
            assert!(errors.iter().any(|e| e.contains("'a'")));
            assert!(errors.iter().any(|e| e.contains("'b'")));
        }
    }
}

#[test]
fn array_literal_elements_are_checked() {
    assert_ok("(defn f [x] [x x])");
    assert_undeclared("(defn f [x] [x y])", "y");
}

#[test]
fn addr_of_and_deref_operands_are_checked() {
    assert_ok("(defn f [p] *p)");
    assert_ok("(defn f [x] &x)");
    assert_undeclared("(defn f [] &y)", "y");
    assert_undeclared("(defn f [] *y)", "y");
}

#[test]
fn index_operands_are_checked() {
    assert_ok("(defn f [arr i] (index arr i))");
    assert_undeclared("(defn f [arr] (index arr i))", "i");
}

#[test]
fn if_branches_are_checked() {
    assert_ok("(defn f [x] (if x x x))");
    assert_undeclared("(defn f [x] (if x y x))", "y");
    assert_undeclared("(defn f [x] (if x x y))", "y");
}
