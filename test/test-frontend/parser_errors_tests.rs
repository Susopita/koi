use koi::frontend::parser::Parser;
use koi::frontend::scanner::Scanner;

fn parse_err(src: &str) -> String {
    match Parser::new(Scanner::new(src)).parse_program() {
        Ok(ast) => panic!("expected `{src}` to fail to parse, got {ast:?}"),
        Err(e) => e,
    }
}

#[test]
fn unclosed_call_reports_expected_rparen() {
    let err = parse_err("(defn add [x y]\n  (+ x y)");
    assert!(err.contains("Expected )"), "unexpected message: {err}");
}

#[test]
fn missing_function_name() {
    let err = parse_err("(defn [x] x)");
    assert!(
        err.to_lowercase().contains("expected function name"),
        "unexpected message: {err}"
    );
}

#[test]
fn missing_parameter_list_brackets() {
    let err = parse_err("(defn f x (+ x 1))");
    assert!(
        err.contains("Expected [") || err.contains("Expected identifier"),
        "unexpected message: {err}"
    );
}

#[test]
fn if_with_no_condition_is_unexpected_token() {
    let err = parse_err("(defn f [] (if))");
    assert!(
        err.contains("Unexpected token"),
        "unexpected message: {err}"
    );
}

#[test]
fn let_missing_bracket_around_bindings() {
    let err = parse_err("(defn f [] (let (a 1) a))");
    assert!(
        err.contains("Expected [ to start let bindings"),
        "unexpected message: {err}"
    );
}

#[test]
fn defstruct_field_missing_type() {
    let err = parse_err("(defstruct P [x])");
    assert!(
        err.contains("Expected identifier"),
        "unexpected message: {err}"
    );
}

#[test]
fn new_missing_type_name_at_eof() {
    let err = parse_err("(defn f [] (new");
    assert!(
        err.contains("Expected identifier") || err.contains("Eof"),
        "unexpected message: {err}"
    );
}

#[test]
fn unexpected_top_level_rparen() {
    let err = parse_err(")");
    assert!(
        err.contains("Unexpected token"),
        "unexpected message: {err}"
    );
}

#[test]
fn error_message_includes_line_and_column() {
    let err = parse_err("(defn f [x]\n  (+ x y)");
    assert!(err.contains("line 2"), "expected line info in: {err}");
}
