use koi_assembly::compile_ir_json_to_assembly;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn simple_program_lowers_to_main_and_direct_call() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [
    {
      "name": "add",
      "returnType": "i64",
      "parameters": [["x", "i64"], ["y", "i64"]],
      "blocks": [{
        "label": "entry",
        "instructions": [
          {"op": "binop", "result": "%v0", "lhs": "x", "rhs": "y", "op_type": "+", "type": "i64"},
          {"op": "return", "value": "%v0"}
        ]
      }]
    },
    {
      "name": "main",
      "returnType": "i64",
      "parameters": [],
      "blocks": [{
        "label": "entry",
        "instructions": [
          {"op": "const", "result": "%v0", "value": 5, "type": "i64"},
          {"op": "const", "result": "%v1", "value": 3, "type": "i64"},
          {"op": "call", "result": "%v2", "function": "add", "arguments": ["%v0", "%v1"], "type": "i64"},
          {"op": "return", "value": "%v2"}
        ]
      }]
    }
  ]
}
"#;

    let asm = compile_ir_json_to_assembly(ir).expect("expected codegen to succeed");
    assert!(asm.contains(".globl main"));
    assert!(asm.contains("add:"));
    assert!(asm.contains("call\tadd"));
    assemble_if_possible(&asm, "simple_program");
}

#[test]
fn phi_nodes_are_lowered_to_edge_moves() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "select",
    "returnType": "i64",
    "parameters": [["cond", "bool"]],
    "blocks": [
      {
        "label": "entry",
        "instructions": [
          {"op": "branch", "cond": "cond", "true_label": "then", "false_label": "else"}
        ]
      },
      {
        "label": "then",
        "instructions": [
          {"op": "const", "result": "%v0", "value": 1, "type": "i64"},
          {"op": "jump", "label": "merge"}
        ]
      },
      {
        "label": "else",
        "instructions": [
          {"op": "const", "result": "%v1", "value": 2, "type": "i64"},
          {"op": "jump", "label": "merge"}
        ]
      },
      {
        "label": "merge",
        "instructions": [
          {"op": "phi", "result": "%v2", "incoming": [["then", "%v0"], ["else", "%v1"]], "type": "i64"},
          {"op": "return", "value": "%v2"}
        ]
      }
    ]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir).expect("expected phi lowering to succeed");
    assert!(!asm.contains("\"phi\""));
    assert!(asm.contains(".Lselect_merge:"));
    assert!(asm.contains(".Lselect_then:"));
    assert!(asm.contains(".Lselect_else:"));
    assemble_if_possible(&asm, "phi_program");
}

#[test]
fn indirect_calls_and_memory_instructions_codegen() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "apply",
    "returnType": "i64",
    "parameters": [["f", "fn_i64_to_i64"], ["x", "i64"], ["p", "Point"]],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "addr_of", "result": "%v0", "operand": "x", "type": "ptr_i64"},
        {"op": "deref", "result": "%v1", "operand": "%v0", "type": "i64"},
        {"op": "get_field", "result": "%v2", "object": "p", "field": "x", "type": "i64"},
        {"op": "alloc", "result": "%v3", "type": "arr_i64", "size": null},
        {"op": "const", "result": "%v4", "value": 0, "type": "i64"},
        {"op": "get_index", "result": "%v5", "array": "%v3", "index": "%v4", "type": "i64"},
        {"op": "call_indirect", "result": "%v6", "function_value": "f", "arguments": ["%v1"], "type": "i64"},
        {"op": "binop", "result": "%v7", "lhs": "%v6", "rhs": "%v2", "op_type": "+", "type": "i64"},
        {"op": "binop", "result": "%v8", "lhs": "%v7", "rhs": "%v5", "op_type": "+", "type": "i64"},
        {"op": "return", "value": "%v8"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir).expect("expected memory/codegen to succeed");
    assert!(asm.contains("call\tmalloc"));
    assert!(asm.contains("call\t*%rax"));
    assert!(asm.contains("leaq"));
    assemble_if_possible(&asm, "memory_program");
}

#[test]
fn loop_phi_back_edge_values_survive_optimization() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "loop_test",
    "returnType": "i64",
    "parameters": [["n", "i64"]],
    "blocks": [
      {
        "label": "entry",
        "instructions": [
          {"op": "const", "result": "%v0", "value": 0, "type": "i64"},
          {"op": "jump", "label": "loop_header"}
        ]
      },
      {
        "label": "loop_header",
        "instructions": [
          {"op": "phi", "result": "%v1", "incoming": [["entry", "%v0"], ["loop_body", "%v4"]], "type": "i64"},
          {"op": "binop", "result": "%v2", "lhs": "%v1", "rhs": "n", "op_type": "<", "type": "bool"},
          {"op": "branch", "cond": "%v2", "true_label": "loop_body", "false_label": "loop_exit"}
        ]
      },
      {
        "label": "loop_body",
        "instructions": [
          {"op": "const", "result": "%v3", "value": 1, "type": "i64"},
          {"op": "binop", "result": "%v4", "lhs": "%v1", "rhs": "%v3", "op_type": "+", "type": "i64"},
          {"op": "jump", "label": "loop_header"}
        ]
      },
      {
        "label": "loop_exit",
        "instructions": [
          {"op": "return", "value": "%v1"}
        ]
      }
    ]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir).expect("expected loop phi lowering to succeed");
    assert!(asm.contains(".Lloop_test_loop_body:"));
    assemble_if_possible(&asm, "loop_phi_program");
}

fn assemble_if_possible(asm: &str, stem: &str) {
    let Some(zig) = zig_binary() else {
        return;
    };

    let base = temp_path(stem);
    let asm_path = base.with_extension("s");
    let obj_path = base.with_extension("o");
    fs::write(&asm_path, asm).expect("expected assembly file to be written");

    let status = Command::new(zig)
        .arg("cc")
        .arg("-c")
        .arg(&asm_path)
        .arg("-o")
        .arg(&obj_path)
        .status()
        .expect("expected zig cc to run");

    assert!(status.success(), "zig cc failed to assemble {asm_path:?}");

    let _ = fs::remove_file(asm_path);
    let _ = fs::remove_file(obj_path);
}

fn zig_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("KOI_ZIG") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let candidate = PathBuf::from("/home/aleu/snap/codex/34/zig-x86_64-linux-0.16.0/zig");
    candidate.is_file().then_some(candidate)
}

fn temp_path(stem: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("koi-assembly-{stem}-{nonce}"))
}
