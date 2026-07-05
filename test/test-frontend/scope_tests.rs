use koi::frontend::parser::Parser;
use koi::frontend::scanner::Scanner;
use koi::frontend::scope::ScopeAnalyzer;

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
fn set_field_field_name_is_not_scope_checked() {
    assert_ok("(defstruct Point [x i64] [y i64]) (defn f [p v] (set-field! p x v))");
}

#[test]
fn set_field_object_and_value_are_checked() {
    assert_undeclared("(defstruct Point [x i64] [y i64]) (defn f [] (set-field! p x 1))", "p");
    assert_undeclared("(defstruct Point [x i64] [y i64]) (defn f [p] (set-field! p x v))", "v");
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

#[test]
fn set_of_undeclared_variable_is_reported() {
    assert_undeclared("(defn f [] (set! x 1))", "x");
}

#[test]
fn set_of_declared_parameter_is_allowed() {
    assert_ok("(defn f [x] (set! x 1))");
}

#[test]
fn set_of_let_binding_is_allowed() {
    assert_ok("(defn f [] (let [a 1] (set! a 2)))");
}

#[test]
fn set_of_loop_variable_is_allowed() {
    assert_ok("(defn f [n] (loop [i 0] (< i n) (+ i 1) (set! i (+ i 1))))");
}

#[test]
fn set_value_expression_is_checked() {
    assert_undeclared("(defn f [x] (set! x y))", "y");
}

#[test]
fn while_condition_and_body_are_checked() {
    assert_ok("(defn f [x] (while (< x 10) (set! x (+ x 1))))");
    assert_undeclared("(defn f [x] (while y (set! x (+ x 1))))", "y");
    assert_undeclared("(defn f [x] (while (< x 10) (set! y 1)))", "y");
}

#[test]
fn while_body_can_reference_outer_variable() {
    assert_ok("(defn f [x y] (while (< x y) (set! x (+ x 1))))");
}

#[test]
fn do_sequences_multiple_checked_expressions() {
    assert_ok("(defn f [x] (do (set! x 1) (set! x 2) x))");
    assert_undeclared("(defn f [x] (do (set! x 1) y))", "y");
}
