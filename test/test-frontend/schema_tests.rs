use koi::frontend::parser::Parser;
use koi::frontend::scanner::Scanner;
use koi::frontend::scope::ScopeAnalyzer;
use std::fs;
use std::path::Path;

/// Every JSON object produced by the AST must carry `nodeType`, and every
/// node except the root `program` must also carry `line`/`column` --
/// this is the "non-negotiable" contract koi-ir depends on.
fn assert_schema(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            assert!(map.contains_key("nodeType"), "missing nodeType in {map:?}");
            if map.get("nodeType") != Some(&serde_json::Value::String("program".to_string())) {
                assert!(map.contains_key("line"), "missing line in {map:?}");
                assert!(map.contains_key("column"), "missing column in {map:?}");
            }
            for v in map.values() {
                assert_schema(v);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                assert_schema(item);
            }
        }
        _ => {}
    }
}

fn compile_to_value(source: &str) -> serde_json::Value {
    let ast = Parser::new(Scanner::new(source))
        .parse_program()
        .unwrap_or_else(|e| panic!("failed to parse: {e}"));

    ScopeAnalyzer::new()
        .analyze(&ast)
        .unwrap_or_else(|errors| panic!("failed to scope-check: {errors:?}"));

    serde_json::to_value(&ast).expect("AST must serialize to JSON")
}

fn test_programs_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("test/casos_prueba_carp")
}

#[test]
fn every_sample_program_produces_schema_valid_json() {
    let dir = test_programs_dir();
    let mut checked = 0;

    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}")) {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("carp") {
            continue;
        }

        let source =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));
        let value = compile_to_value(&source);

        assert_eq!(
            value.get("nodeType"),
            Some(&serde_json::Value::String("program".to_string())),
            "root node in {path:?} must be a program"
        );
        assert_schema(&value);
        checked += 1;
    }

    assert!(
        checked >= 5,
        "expected at least 5 .carp sample programs, found {checked}"
    );
}

#[test]
fn kitchen_sink_program_covers_every_node_kind() {
    let source = fs::read_to_string(test_programs_dir().join("kitchen_sink.carp"))
        .expect("kitchen_sink.carp must exist");
    let value = compile_to_value(&source);
    let dump = serde_json::to_string(&value).unwrap();

    let expected_node_types = [
        "program",
        "struct_def",
        "function_def",
        "call",
        "variable",
        "literal",
        "lambda",
        "let_binding",
        "if",
        "loop",
        "field_access",
        "index",
        "addr_of",
        "deref",
        "new",
        "array_literal",
    ];

    for node_type in expected_node_types {
        assert!(
            dump.contains(&format!("\"nodeType\":\"{node_type}\"")),
            "kitchen_sink.carp never exercises nodeType \"{node_type}\""
        );
    }
}

#[test]
fn literal_type_strings_match_the_contract() {
    let value = compile_to_value("(defn f [] [1 1.5 \"hi\" true])");
    let dump = serde_json::to_string(&value).unwrap();
    for literal_type in ["int64", "float64", "string", "bool"] {
        assert!(
            dump.contains(&format!("\"literalType\":\"{literal_type}\"")),
            "missing literalType \"{literal_type}\" in {dump}"
        );
    }
}

#[test]
fn parameters_serialize_as_json_arrays_not_objects() {
    let value = compile_to_value("(defn add [x y] (+ x y))");
    let params = &value["children"][0]["parameters"];
    assert!(
        params.is_array(),
        "parameters must serialize as a JSON array, got {params:?}"
    );
    assert!(
        params[0].is_array(),
        "each parameter must serialize as a [name, type] pair, got {:?}",
        params[0]
    );
}
