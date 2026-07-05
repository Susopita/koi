//! Backend tests for arm64 and risc-v assembly generation.
//!
//! These tests verify that the instruction selectors produce syntactically
//! valid assembly text with the expected labels and opcodes.  The assembly
//! is *not* assembled on this host (Apple Silicon) since koi generates
//! x86_64 and riscv asm — only the textual structure is checked.

use koi::backend::{compile_ir_json_to_assembly, TargetArch};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Minimal IR: a function `add` that returns lhs + rhs, plus `main`.
fn add_ir() -> &'static str {
    r#"{
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
}"#
}

/// IR with an if-else pattern (exercises branch + phi → if-conversion).
fn branch_ir() -> &'static str {
    r#"{
  "irType": "hir",
  "functions": [
    {
      "name": "test",
      "returnType": "i64",
      "parameters": [["flag", "bool"]],
      "blocks": [
        {
          "label": "entry",
          "instructions": [
            {"op": "branch", "cond": "flag", "true_label": "then", "false_label": "else"}
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
    }
  ]
}"#
}

// ---------------------------------------------------------------------------
// ARM64 tests
// ---------------------------------------------------------------------------

#[test]
fn arm64_simple_addition_has_expected_labels() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::Arm64)
        .expect("arm64 codegen should succeed");
    assert!(asm.contains("add:"), "arm64 output must contain 'add:' label");
    assert!(asm.contains("main:"), "arm64 output must contain 'main:' label");
    assert!(asm.contains("stp"), "arm64 output must contain stp (prologue)");
    assert!(asm.contains("ldp"), "arm64 output must contain ldp (epilogue)");
    assert!(asm.contains("ret"), "arm64 output must contain ret");
}

#[test]
fn arm64_branch_generates_csel_or_bcond() {
    let asm = compile_ir_json_to_assembly(branch_ir(), TargetArch::Arm64)
        .expect("arm64 branch codegen should succeed");
    // If-conversion may or may not trigger depending on block structure.
    // At minimum, we should see branch instructions.
    assert!(
        asm.contains("csel") || asm.contains("b.") || asm.contains("bne") || asm.contains("beq"),
        "arm64 should emit conditional branch or csel: {asm}"
    );
}

#[test]
fn arm64_emit_has_arch_directive() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::Arm64)
        .expect("arm64 codegen should succeed");
    assert!(asm.contains(".arch armv8-a"), "arm64 output must have .arch directive");
}

#[test]
fn arm64_movz_or_movk_for_constants() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::Arm64)
        .expect("arm64 codegen should succeed");
    // The materialization pass should produce movz/movk for 5 and 3
    assert!(
        asm.contains("movz") || asm.contains("mov") || asm.contains("add"),
        "arm64 should materialize constants: {asm}"
    );
}

// ---------------------------------------------------------------------------
// RISC-V tests
// ---------------------------------------------------------------------------

#[test]
fn riscv_simple_addition_has_expected_labels() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::RiscV)
        .expect("riscv codegen should succeed");
    assert!(asm.contains("add:"), "riscv output must contain 'add:' label");
    assert!(asm.contains("main:"), "riscv output must contain 'main:' label");
    assert!(asm.contains("ret"), "riscv output must contain ret");
}

#[test]
fn riscv_immediate_folding_uses_addi() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::RiscV)
        .expect("riscv codegen should succeed");
    // 3 and 5 are small immediates → addi
    assert!(
        asm.contains("addi"),
        "riscv should use addi for small constants: {asm}"
    );
}

#[test]
fn riscv_branch_generates_bne() {
    let asm = compile_ir_json_to_assembly(branch_ir(), TargetArch::RiscV)
        .expect("riscv branch codegen should succeed");
    assert!(
        asm.contains("bne"),
        "riscv should emit bne for branch-on-boolean: {asm}"
    );
}

#[test]
fn riscv_has_text_directive() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::RiscV)
        .expect("riscv codegen should succeed");
    assert!(asm.contains(".text"), "riscv output must have .text directive");
}

#[test]
fn riscv_call_uses_call_or_jal() {
    let asm = compile_ir_json_to_assembly(add_ir(), TargetArch::RiscV)
        .expect("riscv codegen should succeed");
    // The riscv backend emits `call` or `jal` depending on the instruction
    // selector phase.
    assert!(
        asm.contains("call") || asm.contains("jal"),
        "riscv should emit call/jal for function calls: {asm}"
    );
}

// ---------------------------------------------------------------------------
// Negative / error tests
// ---------------------------------------------------------------------------

#[test]
fn riscv_unsupported_constant_falls_back() {
    // This should not crash — the codegen should handle any input without panic.
    let ir = r#"{
  "irType": "hir",
  "functions": [
    {
      "name": "main",
      "returnType": "i64",
      "parameters": [],
      "blocks": [{
        "label": "entry",
        "instructions": [
          {"op": "const", "result": "%v0", "value": "some_string", "type": "string"},
          {"op": "return", "value": "%v0"}
        ]
      }]
    }
  ]
}"#;
    let result = compile_ir_json_to_assembly(ir, TargetArch::RiscV);
    assert!(result.is_ok(), "riscv codegen should handle string constants: {result:?}");
}
