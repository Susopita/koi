use koi::backend::{compile_ir_json_to_assembly, TargetArch};
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

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected codegen to succeed");
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

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected phi lowering to succeed");
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

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected memory/codegen to succeed");
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

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected loop phi lowering to succeed");
    assert!(asm.contains(".Lloop_test_loop_body:"));
    assemble_if_possible(&asm, "loop_phi_program");
}

#[test]
fn mul_by_power_of_two_strength_reduces_to_shift() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "times_eight",
    "returnType": "i64",
    "parameters": [["x", "i64"]],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "const", "result": "%v0", "value": 8, "type": "i64"},
        {"op": "binop", "result": "%v1", "lhs": "x", "rhs": "%v0", "op_type": "*", "type": "i64"},
        {"op": "return", "value": "%v1"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected codegen to succeed");
    assert!(asm.contains("salq"), "expected strength reduction to emit a shift instruction");
    assert!(!asm.contains("imulq"), "expected no imulq for x * 8 after strength reduction");
    assemble_if_possible(&asm, "mul_pow2_program");
}

#[test]
fn div_by_power_of_two_strength_reduces_to_shift() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "div_four",
    "returnType": "i64",
    "parameters": [["x", "i64"]],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "const", "result": "%v0", "value": 4, "type": "i64"},
        {"op": "binop", "result": "%v1", "lhs": "x", "rhs": "%v0", "op_type": "/", "type": "i64"},
        {"op": "return", "value": "%v1"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected codegen to succeed");
    assert!(asm.contains("sarq"), "expected strength reduction to emit a shift instruction");
    assemble_if_possible(&asm, "div_pow2_program");
}

#[test]
fn set_index_then_get_index_stores_and_reloads_at_computed_address() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "set_and_get",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "alloc", "result": "%v0", "type": "arr_i64", "size": null},
        {"op": "const", "result": "%v1", "value": 0, "type": "i64"},
        {"op": "const", "result": "%v2", "value": 42, "type": "i64"},
        {"op": "set_index", "array": "%v0", "index": "%v1", "value": "%v2", "type": "i64"},
        {"op": "get_index", "result": "%v3", "array": "%v0", "index": "%v1", "type": "i64"},
        {"op": "return", "value": "%v3"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected set_index codegen to succeed");
    assert!(asm.contains("call\tmalloc"));
    // `emit_set_index` stores the value into the computed address...
    assert!(
        asm.contains("movq\t%r11, 0(%rax)"),
        "expected a store into the computed array address"
    );
    // ...and the subsequent `get_index` reloads from that same computed
    // address shape (array_ptr + index * element_size) into a register.
    assert!(
        asm.contains("movq\t0(%rax), %r11"),
        "expected a load from the computed array address"
    );
    assemble_if_possible(&asm, "set_index_program");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn set_field_then_get_field_stores_and_reloads_at_computed_offset() {
    // Simulates building the actual "Closure" struct the koi-ir side will
    // emit for closures-with-capture: a 2-field struct with "fn_ptr" and
    // "env_ptr", built via `alloc` + `set_field`, then read back via
    // `get_field`. Exercises `emit_set_field` (the write counterpart to
    // `emit_get_field`) and confirms round-trip correctness by actually
    // executing the compiled program via gcc, not just inspecting asm text.
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "main",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "alloc", "result": "%v0", "type": "Closure", "size": null},
        {"op": "const", "result": "%v1", "value": 111, "type": "i64"},
        {"op": "const", "result": "%v2", "value": 222, "type": "i64"},
        {"op": "set_field", "object": "%v0", "field": "fn_ptr", "value": "%v1", "type": "i64"},
        {"op": "set_field", "object": "%v0", "field": "env_ptr", "value": "%v2", "type": "i64"},
        {"op": "get_field", "result": "%v3", "object": "%v0", "field": "fn_ptr", "type": "i64"},
        {"op": "get_field", "result": "%v4", "object": "%v0", "field": "env_ptr", "type": "i64"},
        {"op": "binop", "result": "%v5", "lhs": "%v3", "rhs": "%v4", "op_type": "+", "type": "i64"},
        {"op": "call", "function": "print", "arguments": ["%v5"], "type": null},
        {"op": "const", "result": "%v6", "value": 0, "type": "i64"},
        {"op": "return", "value": "%v6"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected set_field codegen to succeed");
    assert!(asm.contains("call\tmalloc"));
    // `emit_set_field` stores each value into `offset(object_ptr)` -- the
    // "fn_ptr" field is discovered first (offset 0) and "env_ptr" second
    // (offset 8), mirroring the `set_index`/`get_index` test's assertion
    // style for computed-address stores/loads.
    assert!(
        asm.contains("movq\t%r10, 0(%rax)"),
        "expected a store into fn_ptr at offset 0"
    );
    assert!(
        asm.contains("movq\t%r10, 8(%rax)"),
        "expected a store into env_ptr at offset 8"
    );
    assert!(
        asm.contains("movq\t0(%rax), %r10"),
        "expected a load from fn_ptr at offset 0"
    );
    assert!(
        asm.contains("movq\t8(%rax), %r10"),
        "expected a load from env_ptr at offset 8"
    );

    let stdout = assemble_link_and_run(&asm, "set_field_program");
    assert_eq!(stdout.trim(), "333", "expected fn_ptr (111) + env_ptr (222) round-tripped through the struct to equal 333");
    assemble_if_possible(&asm, "set_field_program_zig");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn write_only_field_gets_its_own_offset_not_offset_zero() {
    // Regression test for the `Layouts::from_program` offset-discovery fix:
    // before scanning `SetField` too, a field written but never read (like
    // "a" here) would get no entry in `field_offsets` at all, and
    // `field_offset`'s fallback would silently return 0 for it -- aliasing
    // whatever field legitimately owns offset 0. Here "b" is read via
    // `get_field` (discovered first, offset 0) and "a" is only ever
    // *written* via `set_field` in the same function (must be discovered
    // too, and must land at a different, non-overlapping offset: 8).
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "main",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "alloc", "result": "%v0", "type": "Pair", "size": null},
        {"op": "const", "result": "%v1", "value": 7, "type": "i64"},
        {"op": "const", "result": "%v2", "value": 9, "type": "i64"},
        {"op": "set_field", "object": "%v0", "field": "b", "value": "%v1", "type": "i64"},
        {"op": "set_field", "object": "%v0", "field": "a", "value": "%v2", "type": "i64"},
        {"op": "get_field", "result": "%v3", "object": "%v0", "field": "b", "type": "i64"},
        {"op": "call", "function": "print", "arguments": ["%v3"], "type": null},
        {"op": "const", "result": "%v4", "value": 0, "type": "i64"},
        {"op": "return", "value": "%v4"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected write-only field codegen to succeed");
    // "b" is the first (struct_type, field) pair encountered in instruction
    // order (via its `set_field`, before its own later `get_field`), so it
    // must land at offset 0; "a"'s `set_field` comes second and, if the
    // fix works, must be assigned the *next* offset (8) rather than
    // silently defaulting to 0 and aliasing "b".
    assert!(
        asm.contains("movq\t%r10, 0(%rax)"),
        "expected field 'b' (first discovered) to be stored at offset 0"
    );
    assert!(
        asm.contains("movq\t%r10, 8(%rax)"),
        "expected write-only field 'a' to be assigned its own offset (8), not alias offset 0"
    );

    let stdout = assemble_link_and_run(&asm, "write_only_field_program");
    // If "a"'s write-only field wrongly aliased offset 0, its store would
    // clobber "b" and this would print 9 instead of 7.
    assert_eq!(stdout.trim(), "7", "expected 'b' to read back as 7, unclobbered by the write-only field 'a'");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn f64_const_and_print_produces_correct_stdout() {
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "main",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "const", "result": "%v0", "value": 3.75, "type": "f64"},
        {"op": "call", "function": "print", "arguments": ["%v0"], "type": null},
        {"op": "const", "result": "%v1", "value": 0, "type": "i64"},
        {"op": "return", "value": "%v1"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected f64 const codegen to succeed");
    assert!(asm.contains("movsd"), "expected a movsd instruction for the f64 constant");
    assert!(asm.contains(".double"), "expected an interned .double literal");
    let stdout = assemble_link_and_run(&asm, "f64_const_print");
    assert_eq!(stdout.trim(), "3.750000");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn f64_arithmetic_add_sub_mul_div_produce_correct_results() {
    // (+ 1.5 2.25) = 3.75
    let add_ir = binop_print_program("+", 1.5, 2.25);
    let stdout = assemble_link_and_run(
        &compile_ir_json_to_assembly(&add_ir, TargetArch::X8664).expect("expected f64 add codegen to succeed"),
        "f64_add",
    );
    assert_eq!(stdout.trim(), "3.750000");

    // (- 5.5 2.25) = 3.25
    let sub_ir = binop_print_program("-", 5.5, 2.25);
    let stdout = assemble_link_and_run(
        &compile_ir_json_to_assembly(&sub_ir, TargetArch::X8664).expect("expected f64 sub codegen to succeed"),
        "f64_sub",
    );
    assert_eq!(stdout.trim(), "3.250000");

    // (* 2.5 4.0) = 10.0
    let mul_ir = binop_print_program("*", 2.5, 4.0);
    let stdout = assemble_link_and_run(
        &compile_ir_json_to_assembly(&mul_ir, TargetArch::X8664).expect("expected f64 mul codegen to succeed"),
        "f64_mul",
    );
    assert_eq!(stdout.trim(), "10.000000");

    // (/ 7.5 2.5) = 3.0
    let div_ir = binop_print_program("/", 7.5, 2.5);
    let stdout = assemble_link_and_run(
        &compile_ir_json_to_assembly(&div_ir, TargetArch::X8664).expect("expected f64 div codegen to succeed"),
        "f64_div",
    );
    assert_eq!(stdout.trim(), "3.000000");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn f64_comparison_produces_correct_boolean() {
    // (< 1.5 2.5) is true -> prints the i64 1; a float binop producing a
    // bool, consumed (and printed) via the ordinary i64 print path.
    let ir = r#"
{
  "irType": "hir",
  "functions": [{
    "name": "main",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "const", "result": "%v0", "value": 1.5, "type": "f64"},
        {"op": "const", "result": "%v1", "value": 2.5, "type": "f64"},
        {"op": "binop", "result": "%v2", "lhs": "%v0", "rhs": "%v1", "op_type": "<", "type": "bool"},
        {"op": "call", "result": "%v3", "function": "select_int", "arguments": ["%v2"], "type": "i64"},
        {"op": "call", "function": "print", "arguments": ["%v3"], "type": null},
        {"op": "const", "result": "%v4", "value": 0, "type": "i64"},
        {"op": "return", "value": "%v4"}
      ]
    }]
  }, {
    "name": "select_int",
    "returnType": "i64",
    "parameters": [["cond", "bool"]],
    "blocks": [{
      "label": "entry",
      "instructions": [
        {"op": "branch", "cond": "cond", "true_label": "then", "false_label": "else"}
      ]
    }, {
      "label": "then",
      "instructions": [
        {"op": "const", "result": "%r0", "value": 1, "type": "i64"},
        {"op": "return", "value": "%r0"}
      ]
    }, {
      "label": "else",
      "instructions": [
        {"op": "const", "result": "%r1", "value": 0, "type": "i64"},
        {"op": "return", "value": "%r1"}
      ]
    }]
  }]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected f64 comparison codegen to succeed");
    assert!(asm.contains("ucomisd"), "expected a ucomisd instruction for the f64 comparison");
    let stdout = assemble_link_and_run(&asm, "f64_compare");
    assert_eq!(stdout.trim(), "1", "expected (< 1.5 2.5) to be true (prints 1)");
}

#[test]
#[cfg_attr(not(target_arch = "x86_64"), ignore)]
fn f64_parameter_and_return_value_round_trip_through_a_call() {
    // `double_it(x: f64) -> f64 { x * 2.0 }`, called from main with 4.5,
    // printed by the caller -> expects 9.000000. Exercises both
    // argument-passing (XMM parameter home) and return-value handling
    // (FLOAT_RETURN_REGISTER) for a user-defined function.
    let ir = r#"
{
  "irType": "hir",
  "functions": [
    {
      "name": "double_it",
      "returnType": "f64",
      "parameters": [["x", "f64"]],
      "blocks": [{
        "label": "entry",
        "instructions": [
          {"op": "const", "result": "%v0", "value": 2.0, "type": "f64"},
          {"op": "binop", "result": "%v1", "lhs": "x", "rhs": "%v0", "op_type": "*", "type": "f64"},
          {"op": "return", "value": "%v1"}
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
          {"op": "const", "result": "%v0", "value": 4.5, "type": "f64"},
          {"op": "call", "result": "%v1", "function": "double_it", "arguments": ["%v0"], "type": "f64"},
          {"op": "call", "function": "print", "arguments": ["%v1"], "type": null},
          {"op": "const", "result": "%v2", "value": 0, "type": "i64"},
          {"op": "return", "value": "%v2"}
        ]
      }]
    }
  ]
}
"#;

    let asm = compile_ir_json_to_assembly(ir, TargetArch::X8664).expect("expected f64 param/return codegen to succeed");
    let stdout = assemble_link_and_run(&asm, "f64_param_return");
    assert_eq!(stdout.trim(), "9.000000");
}

/// Builds a one-function `main` IR program computing `lhs op_type rhs` as
/// an f64 binop and printing the result.
fn binop_print_program(op_type: &str, lhs: f64, rhs: f64) -> String {
    format!(
        r#"
{{
  "irType": "hir",
  "functions": [{{
    "name": "main",
    "returnType": "i64",
    "parameters": [],
    "blocks": [{{
      "label": "entry",
      "instructions": [
        {{"op": "const", "result": "%v0", "value": {lhs}, "type": "f64"}},
        {{"op": "const", "result": "%v1", "value": {rhs}, "type": "f64"}},
        {{"op": "binop", "result": "%v2", "lhs": "%v0", "rhs": "%v1", "op_type": "{op_type}", "type": "f64"}},
        {{"op": "call", "function": "print", "arguments": ["%v2"], "type": null}},
        {{"op": "const", "result": "%v3", "value": 0, "type": "i64"}},
        {{"op": "return", "value": "%v3"}}
      ]
    }}]
  }}]
}}
"#
    )
}

/// Real correctness check (not just "assembles without error"): writes the
/// generated assembly to a temp file, assembles+links it with the system
/// `gcc` (this environment doesn't have the `zig`-gated toolchain the other
/// tests in this file optionally use), executes the resulting binary, and
/// returns its captured stdout. Panics on any failure along the way.
///
/// On non-x86-64 hosts the assembly step is skipped (koi generates x86-64
/// code which cannot be assembled on other architectures) and an empty string
/// is returned.
fn assemble_link_and_run(asm: &str, stem: &str) -> String {
    if cfg!(not(target_arch = "x86_64")) {
        return String::new();
    }

    let base = temp_path(stem);
    let asm_path = base.with_extension("s");
    let obj_path = base.with_extension("o");
    let bin_path = base.with_extension("bin");
    fs::write(&asm_path, asm).expect("expected assembly file to be written");

    let compile_status = Command::new("gcc")
        .arg("-c")
        .arg(&asm_path)
        .arg("-o")
        .arg(&obj_path)
        .status()
        .expect("expected gcc -c to run");
    assert!(compile_status.success(), "gcc -c failed to assemble {asm_path:?}");

    let link_status = Command::new("gcc")
        .arg(&obj_path)
        .arg("-o")
        .arg(&bin_path)
        .status()
        .expect("expected gcc link to run");
    assert!(link_status.success(), "gcc failed to link {obj_path:?}");

    let output = Command::new(&bin_path)
        .output()
        .expect("expected compiled binary to run");
    assert!(output.status.success(), "compiled binary {bin_path:?} exited with failure");

    let _ = fs::remove_file(&asm_path);
    let _ = fs::remove_file(&obj_path);
    let _ = fs::remove_file(&bin_path);

    String::from_utf8(output.stdout).expect("expected program stdout to be valid UTF-8")
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
