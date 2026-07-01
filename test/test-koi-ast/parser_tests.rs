use koi_ast::ast::ASTNode;
use koi_ast::parser::Parser;
use koi_ast::scanner::Scanner;

fn parse(src: &str) -> ASTNode {
    Parser::new(Scanner::new(src))
        .parse_program()
        .unwrap_or_else(|e| panic!("expected `{src}` to parse, got error: {e}"))
}

fn only_child(program: &ASTNode) -> &ASTNode {
    match program {
        ASTNode::Program { children } => {
            assert_eq!(children.len(), 1, "expected exactly one top-level form");
            &children[0]
        }
        other => panic!("expected Program, got {other:?}"),
    }
}

#[test]
fn empty_program_has_no_children() {
    let program = parse("");
    match program {
        ASTNode::Program { children } => assert!(children.is_empty()),
        other => panic!("expected Program, got {other:?}"),
    }
}

#[test]
fn function_def_without_type_annotations() {
    let program = parse("(defn add [x y] (+ x y))");
    match only_child(&program) {
        ASTNode::FunctionDef {
            name,
            parameters,
            line,
            column,
            ..
        } => {
            assert_eq!(name, "add");
            assert_eq!(
                parameters,
                &vec![("x".to_string(), None), ("y".to_string(), None)]
            );
            assert_eq!(*line, 1);
            assert_eq!(*column, 1);
        }
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn function_def_with_type_annotations() {
    let program = parse("(defn add [x : i64 y : i64] (+ x y))");
    match only_child(&program) {
        ASTNode::FunctionDef { parameters, .. } => {
            assert_eq!(
                parameters,
                &vec![
                    ("x".to_string(), Some("i64".to_string())),
                    ("y".to_string(), Some("i64".to_string())),
                ]
            );
        }
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn kebab_case_function_name() {
    let program = parse("(defn get-x [p] (field p x))");
    match only_child(&program) {
        ASTNode::FunctionDef { name, .. } => assert_eq!(name, "get-x"),
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn call_captures_line_and_column_at_opening_paren() {
    // Regression: the spec's own example captured line/column *after*
    // parsing the whole call, at the closing paren. It must be captured at
    // the opening '(' instead.
    let program = parse("(defn f [x]\n  (+ x 1))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::Call { line, column, .. } => {
                assert_eq!(*line, 2);
                assert_eq!(*column, 3);
            }
            other => panic!("expected Call, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn array_literal_captures_line_and_column_at_opening_bracket() {
    let program = parse("(defn f []\n  [1 2 3])");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::ArrayLiteral {
                line,
                column,
                elements,
            } => {
                assert_eq!(*line, 2);
                assert_eq!(*column, 3);
                assert_eq!(elements.len(), 3);
            }
            other => panic!("expected ArrayLiteral, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn literal_types_and_values() {
    let program = parse("(defn f [] [1 1.5 \"hi\" true false])");
    let elements = match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::ArrayLiteral { elements, .. } => elements.clone(),
            other => panic!("expected ArrayLiteral, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    };

    let kinds: Vec<(&str, serde_json::Value)> = elements
        .iter()
        .map(|node| match node {
            ASTNode::Literal {
                literal_type,
                value,
                ..
            } => (literal_type.as_str(), value.clone()),
            other => panic!("expected Literal, got {other:?}"),
        })
        .collect();

    assert_eq!(
        kinds,
        vec![
            ("int64", serde_json::json!(1)),
            ("float64", serde_json::json!(1.5)),
            ("string", serde_json::json!("hi")),
            ("bool", serde_json::json!(true)),
            ("bool", serde_json::json!(false)),
        ]
    );
}

#[test]
fn if_with_else() {
    let program = parse("(defn f [x] (if x 1 2))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::IfExpr { else_branch, .. } => assert!(else_branch.is_some()),
            other => panic!("expected IfExpr, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn if_without_else() {
    let program = parse("(defn f [x] (if x 1))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::IfExpr { else_branch, .. } => assert!(else_branch.is_none()),
            other => panic!("expected IfExpr, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn let_bindings_are_sequential_and_ordered() {
    let program = parse("(defn f [] (let [a 1 b (+ a 1)] b))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::LetBinding { bindings, .. } => {
                assert_eq!(bindings.len(), 2);
                assert_eq!(bindings[0].0, "a");
                assert_eq!(bindings[1].0, "b");
            }
            other => panic!("expected LetBinding, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn loop_form_fields() {
    let program = parse("(defn f [n] (loop [i 0] (< i n) (+ i 1) i))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::LoopExpr { variable, .. } => assert_eq!(variable, "i"),
            other => panic!("expected LoopExpr, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn lambda_parameters_and_body() {
    let program = parse("(defn f [] (lambda [y] (+ y 1)))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::Lambda { parameters, .. } => {
                assert_eq!(parameters, &vec![("y".to_string(), None)]);
            }
            other => panic!("expected Lambda, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn defstruct_fields() {
    let program = parse("(defstruct Point [x i64] [y i64])");
    match only_child(&program) {
        ASTNode::StructDef { name, fields, .. } => {
            assert_eq!(name, "Point");
            assert_eq!(
                fields,
                &vec![
                    ("x".to_string(), "i64".to_string()),
                    ("y".to_string(), "i64".to_string())
                ]
            );
        }
        other => panic!("expected StructDef, got {other:?}"),
    }
}

#[test]
fn field_access_field_name_is_raw_not_a_variable_node() {
    let program = parse("(defn f [p] (field p x))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::FieldAccess { field, object, .. } => {
                assert_eq!(field, "x");
                assert!(matches!(object.as_ref(), ASTNode::Variable { name, .. } if name == "p"));
            }
            other => panic!("expected FieldAccess, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn index_form() {
    let program = parse("(defn f [arr i] (index arr i))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => {
            assert!(matches!(body.as_ref(), ASTNode::Index { .. }));
        }
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn index_nested_for_2d_arrays() {
    let program = parse("(defn f [arr i j] (index (index arr i) j))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::Index { array, .. } => {
                assert!(matches!(array.as_ref(), ASTNode::Index { .. }));
            }
            other => panic!("expected Index, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn new_without_size_or_init() {
    let program = parse("(defn f [] (new Point))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::New {
                type_str,
                size_or_init,
                ..
            } => {
                assert_eq!(type_str, "Point");
                assert!(size_or_init.is_none());
            }
            other => panic!("expected New, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn new_with_size_or_init() {
    let program = parse("(defn f [] (new i64 10))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::New { size_or_init, .. } => assert!(size_or_init.is_some()),
            other => panic!("expected New, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn array_literal_2d_nests_correctly() {
    let program = parse("(defn f [] [[1 2] [3 4]])");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::ArrayLiteral { elements, .. } => {
                assert_eq!(elements.len(), 2);
                for row in elements {
                    assert!(matches!(row, ASTNode::ArrayLiteral { .. }));
                }
            }
            other => panic!("expected ArrayLiteral, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn prefix_addr_of() {
    let program = parse("(defn f [x] &x)");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::AddrOf { operand, .. } => {
                assert!(matches!(operand.as_ref(), ASTNode::Variable { name, .. } if name == "x"));
            }
            other => panic!("expected AddrOf, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn prefix_deref() {
    let program = parse("(defn f [x] *x)");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::Deref { operand, .. } => {
                assert!(matches!(operand.as_ref(), ASTNode::Variable { name, .. } if name == "x"));
            }
            other => panic!("expected Deref, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn multiply_call_head_is_variable_star_not_deref() {
    // Regression: '*' in call-function position means the binary multiply
    // builtin, not "dereference the rest of the call".
    let program = parse("(defn f [a b] (* a b))");
    match only_child(&program) {
        ASTNode::FunctionDef { body, .. } => match body.as_ref() {
            ASTNode::Call {
                function,
                arguments,
                ..
            } => {
                assert!(matches!(function.as_ref(), ASTNode::Variable { name, .. } if name == "*"));
                assert_eq!(arguments.len(), 2);
            }
            other => panic!("expected Call, got {other:?}"),
        },
        other => panic!("expected FunctionDef, got {other:?}"),
    }
}

#[test]
fn top_level_bare_call_without_defn_wrapper() {
    let program = parse("(add 1 2)");
    match only_child(&program) {
        ASTNode::Call {
            function,
            arguments,
            ..
        } => {
            assert!(matches!(function.as_ref(), ASTNode::Variable { name, .. } if name == "add"));
            assert_eq!(arguments.len(), 2);
        }
        other => panic!("expected Call, got {other:?}"),
    }
}

#[test]
fn multiple_top_level_forms() {
    let program = parse("(defn a [] 1) (defn b [] 2)");
    match program {
        ASTNode::Program { children } => assert_eq!(children.len(), 2),
        other => panic!("expected Program, got {other:?}"),
    }
}
