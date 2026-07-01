# **🦀 PROMPT C: RUST CRATE KOI-ASSEMBLY — x86-64 Codegen, Register Allocation, Optimizations**

**Responsabilidad:** Implementar code generation x86-64 AT&T, register allocation, y optimizaciones en **Rust**.

**Entrada:** `/tmp/ir.json` (IR desde Persona B)

**Salida:** `output.s` (x86-64 assembly)

**Timeline:** Días 3-7 de 1 semana

**Crate:** `koi-assembly` (binario)

---

## **Estructura del Crate**

```
koi-assembly/
├── Cargo.toml
└── src/
    ├── main.rs                (entry point, lee IR JSON)
    ├── ir_parser.rs           (deserializa /tmp/ir.json)
    ├── register_allocator.rs  (linear scan)
    ├── codegen.rs             (x86-64 AT&T generation)
    ├── optimizer.rs           (DCE, constant fold, peephole)
    └── abi.rs                 (System V AMD64 calling convention)
```

---

## **Parte 1: IR Parser (ir_parser.rs)**

```rust
// koi-assembly/src/ir_parser.rs

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct IRProgram {
    pub functions: Vec<IRFunction>,
}

#[derive(Debug, Clone)]
pub struct IRFunction {
    pub name: String,
    pub return_type: String,
    pub parameters: Vec<(String, String)>,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub label: String,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Clone)]
pub enum Instruction {
    Const {
        result: String,
        value: i64,
        ty: String,
    },
    BinOp {
        result: String,
        lhs: String,
        rhs: String,
        op_type: String,
        ty: String,
    },
    Call {
        result: Option<String>,
        function: String,
        arguments: Vec<String>,
        ty: Option<String>,
    },
    Return {
        value: Option<String>,
    },
    Jump {
        label: String,
    },
    Branch {
        cond: String,
        true_label: String,
        false_label: String,
    },
}

pub struct IRParser;

impl IRParser {
    pub fn parse_json(json_str: &str) -> Result<IRProgram, String> {
        let json: Value = serde_json::from_str(json_str)
            .map_err(|e| format!("JSON parse error: {}", e))?;
        
        let mut functions = vec![];
        
        if let Some(funcs) = json["functions"].as_array() {
            for func_val in funcs {
                functions.push(Self::parse_function(func_val)?);
            }
        }
        
        Ok(IRProgram { functions })
    }
    
    fn parse_function(func_val: &Value) -> Result<IRFunction, String> {
        let name = func_val["name"]
            .as_str()
            .ok_or("Missing function name")?
            .to_string();
        
        let return_type = func_val["returnType"]
            .as_str()
            .unwrap_or("i64")
            .to_string();
        
        let mut parameters = vec![];
        if let Some(params) = func_val["parameters"].as_array() {
            for param in params {
                if let (Some(pname), Some(pty)) = (param[0].as_str(), param[1].as_str()) {
                    parameters.push((pname.to_string(), pty.to_string()));
                }
            }
        }
        
        let mut blocks = vec![];
        if let Some(blks) = func_val["blocks"].as_array() {
            for blk in blks {
                blocks.push(Self::parse_block(blk)?);
            }
        }
        
        Ok(IRFunction {
            name,
            return_type,
            parameters,
            blocks,
        })
    }
    
    fn parse_block(blk: &Value) -> Result<BasicBlock, String> {
        let label = blk["label"]
            .as_str()
            .ok_or("Missing block label")?
            .to_string();
        
        let mut instructions = vec![];
        if let Some(instrs) = blk["instructions"].as_array() {
            for instr in instrs {
                instructions.push(Self::parse_instruction(instr)?);
            }
        }
        
        Ok(BasicBlock { label, instructions })
    }
    
    fn parse_instruction(instr: &Value) -> Result<Instruction, String> {
        let op = instr["op"].as_str().ok_or("Missing op")?;
        
        match op {
            "const" => {
                let result = instr["result"].as_str().unwrap_or("").to_string();
                let value = instr["value"].as_i64().unwrap_or(0);
                let ty = instr["type"].as_str().unwrap_or("i64").to_string();
                
                Ok(Instruction::Const { result, value, ty })
            }
            "binop" => {
                let result = instr["result"].as_str().unwrap_or("").to_string();
                let lhs = instr["lhs"].as_str().unwrap_or("").to_string();
                let rhs = instr["rhs"].as_str().unwrap_or("").to_string();
                let op_type = instr["op_type"].as_str().unwrap_or("+").to_string();
                let ty = instr["type"].as_str().unwrap_or("i64").to_string();
                
                Ok(Instruction::BinOp {
                    result,
                    lhs,
                    rhs,
                    op_type,
                    ty,
                })
            }
            "call" => {
                let result = instr["result"].as_str().map(|s| s.to_string());
                let function = instr["function"].as_str().unwrap_or("").to_string();
                let arguments = instr["arguments"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                let ty = instr["type"].as_str().map(|s| s.to_string());
                
                Ok(Instruction::Call {
                    result,
                    function,
                    arguments,
                    ty,
                })
            }
            "return" => {
                let value = instr["value"].as_str().map(|s| s.to_string());
                Ok(Instruction::Return { value })
            }
            "jump" => {
                let label = instr["label"].as_str().unwrap_or("").to_string();
                Ok(Instruction::Jump { label })
            }
            "branch" => {
                let cond = instr["cond"].as_str().unwrap_or("").to_string();
                let true_label = instr["true_label"].as_str().unwrap_or("").to_string();
                let false_label = instr["false_label"].as_str().unwrap_or("").to_string();
                
                Ok(Instruction::Branch {
                    cond,
                    true_label,
                    false_label,
                })
            }
            _ => Err(format!("Unknown instruction: {}", op)),
        }
    }
}
```

---

## **Parte 2: ABI (System V AMD64) — abi.rs**

```rust
// koi-assembly/src/abi.rs

pub struct AMD64ABI;

impl AMD64ABI {
    // Argument registers (System V AMD64 ABI)
    pub const ARG_REGISTERS: &'static [&'static str] = &["rdi", "rsi", "rdx", "rcx", "r8", "r9"];
    
    // Caller-saved (volatile)
    pub const VOLATILE_REGISTERS: &'static [&'static str] =
        &["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"];
    
    // Callee-saved (non-volatile)
    pub const NONVOLATILE_REGISTERS: &'static [&'static str] =
        &["rbx", "r12", "r13", "r14", "r15"];
    
    // Return value register
    pub const RETURN_REGISTER: &'static str = "rax";
    
    pub fn get_arg_register(index: usize) -> Option<&'static str> {
        if index < Self::ARG_REGISTERS.len() {
            Some(Self::ARG_REGISTERS[index])
        } else {
            None
        }
    }
    
    pub fn stack_offset_for_arg(index: usize) -> Option<i64> {
        if index >= Self::ARG_REGISTERS.len() {
            Some(16 + 8 * (index - Self::ARG_REGISTERS.len() as usize) as i64)
        } else {
            None
        }
    }
}
```

---

## **Parte 3: Register Allocator (Linear Scan) — register_allocator.rs**

```rust
// koi-assembly/src/register_allocator.rs

use crate::ir_parser::{Instruction, IRFunction};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct LiveInterval {
    pub var: String,
    pub start: usize,
    pub end: usize,
    pub assigned_register: Option<String>,
}

pub struct LinearScanAllocator {
    available_registers: Vec<String>,
}

impl LinearScanAllocator {
    pub fn new() -> Self {
        // Use volatile registers for allocation
        let available_registers = vec![
            "r10", "r11", "r9", "r8", "rcx", "rdx", "rsi", "rdi",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
        
        LinearScanAllocator {
            available_registers,
        }
    }
    
    pub fn allocate(&mut self, function: &IRFunction) -> HashMap<String, String> {
        // 1. Compute live intervals for each variable
        let live_intervals = self.compute_live_intervals(function);
        
        // 2. Sort intervals by start point
        let mut sorted = live_intervals;
        sorted.sort_by_key(|li| li.start);
        
        // 3. Linear scan allocation
        let mut allocation = HashMap::new();
        let mut active: Vec<LiveInterval> = vec![];
        
        for interval in sorted {
            // Remove expired intervals
            active.retain(|li| li.end >= interval.start);
            
            if active.len() < self.available_registers.len() {
                // Free register available
                let reg = self.available_registers[active.len()].clone();
                allocation.insert(interval.var.clone(), reg);
                
                let mut new_interval = interval;
                new_interval.assigned_register = Some(allocation[&new_interval.var].clone());
                active.push(new_interval);
            } else {
                // Spill: use stack location
                let stack_offset = active.len() * 8;
                allocation.insert(
                    interval.var.clone(),
                    format!("-{}(%rbp)", stack_offset),
                );
            }
        }
        
        allocation
    }
    
    fn compute_live_intervals(&self, _function: &IRFunction) -> Vec<LiveInterval> {
        // Simplified: assign sequential live intervals
        vec![
            LiveInterval {
                var: "%v0".to_string(),
                start: 0,
                end: 5,
                assigned_register: None,
            },
        ]
    }
}
```

---

## **Parte 4: Code Generation (codegen.rs)**

```rust
// koi-assembly/src/codegen.rs

use crate::ir_parser::{Instruction, IRFunction, IRProgram};
use std::collections::HashMap;

pub struct X86Generator {
    output: String,
    var_allocation: HashMap<String, String>,
    stack_offset: i64,
}

impl X86Generator {
    pub fn new() -> Self {
        X86Generator {
            output: String::new(),
            var_allocation: HashMap::new(),
            stack_offset: 0,
        }
    }
    
    pub fn generate(&mut self, program: &IRProgram) -> String {
        self.emit_preamble();
        
        for func in &program.functions {
            self.generate_function(func);
        }
        
        self.emit_postamble();
        self.output.clone()
    }
    
    fn emit_preamble(&mut self) {
        self.emit_line(".data");
        self.emit_line("print_fmt: .string \"%ld\\n\"");
        self.emit_line("");
        self.emit_line(".text");
        self.emit_line(".globl main");
    }
    
    fn emit_postamble(&mut self) {
        self.emit_line(".section .note.GNU-stack,\"\",@progbits");
    }
    
    fn generate_function(&mut self, func: &IRFunction) {
        self.emit_line(&format!("{}:", func.name));
        
        // Function prologue
        self.emit_instr("push", &["rbp"]);
        self.emit_instr("mov", &["rsp", "rbp"]);
        
        // Calculate stack space needed
        let stack_space = self.calculate_stack_space(func);
        if stack_space > 0 {
            self.emit_instr("sub", &[&format!("${}", stack_space), "rsp"]);
        }
        
        // Generate code for each block
        for block in &func.blocks {
            self.emit_line(&format!(".L{}:", block.label));
            
            for instr in &block.instructions {
                self.generate_instruction(instr);
            }
        }
        
        // Function epilogue
        self.emit_line(&format!(".end_{}:", func.name));
        self.emit_instr("leave", &[]);
        self.emit_instr("ret", &[]);
        self.emit_line("");
    }
    
    fn generate_instruction(&mut self, instr: &Instruction) {
        match instr {
            Instruction::Const { result, value, .. } => {
                let reg = self.get_reg_for_var(result);
                self.emit_instr("mov", &[&format!("${}", value), &reg]);
            }
            Instruction::BinOp {
                result,
                lhs,
                rhs,
                op_type,
                ..
            } => {
                let lhs_reg = self.get_reg_for_var(lhs);
                let rhs_reg = self.get_reg_for_var(rhs);
                let result_reg = self.get_reg_for_var(result);
                
                // Move lhs to result
                self.emit_instr("mov", &[&lhs_reg, "rax"]);
                
                // Perform operation
                match op_type.as_str() {
                    "+" => self.emit_instr("add", &[&rhs_reg, "rax"]),
                    "-" => self.emit_instr("sub", &[&rhs_reg, "rax"]),
                    "*" => {
                        self.emit_instr("imul", &[&rhs_reg, "rax"]);
                    }
                    "/" => {
                        self.emit_instr("cqo", &[]);
                        self.emit_instr("idiv", &[&rhs_reg]);
                    }
                    _ => {}
                }
                
                // Move result
                self.emit_instr("mov", &["rax", &result_reg]);
            }
            Instruction::Call {
                result,
                function,
                arguments,
                ..
            } => {
                // Set up arguments in registers (System V ABI)
                for (i, arg) in arguments.iter().enumerate() {
                    if i < 6 {
                        let arg_reg = ["rdi", "rsi", "rdx", "rcx", "r8", "r9"][i];
                        let var_reg = self.get_reg_for_var(arg);
                        self.emit_instr("mov", &[&var_reg, arg_reg]);
                    }
                }
                
                // Call function
                self.emit_instr("call", &[function]);
                
                // Store result if needed
                if let Some(res) = result {
                    let res_reg = self.get_reg_for_var(res);
                    self.emit_instr("mov", &["rax", &res_reg]);
                }
            }
            Instruction::Return { value } => {
                if let Some(val) = value {
                    let val_reg = self.get_reg_for_var(val);
                    self.emit_instr("mov", &[&val_reg, "rax"]);
                } else {
                    self.emit_instr("xor", &["rax", "rax"]);
                }
            }
            Instruction::Jump { label } => {
                self.emit_instr("jmp", &[&format!(".L{}", label)]);
            }
            Instruction::Branch {
                cond,
                true_label,
                false_label,
            } => {
                let cond_reg = self.get_reg_for_var(cond);
                self.emit_instr("cmp", &["0", &cond_reg]);
                self.emit_instr("jne", &[&format!(".L{}", true_label)]);
                self.emit_instr("jmp", &[&format!(".L{}", false_label)]);
            }
        }
    }
    
    fn get_reg_for_var(&mut self, var: &str) -> String {
        if let Some(reg) = self.var_allocation.get(var) {
            reg.clone()
        } else {
            // Allocate new register (simplified)
            let reg = format!("%v{}", self.var_allocation.len());
            self.var_allocation.insert(var.to_string(), reg.clone());
            reg
        }
    }
    
    fn calculate_stack_space(&self, _func: &IRFunction) -> i64 {
        // Simplified: allocate space for variables
        // In real implementation, use register allocator
        16
    }
    
    fn emit_instr(&mut self, op: &str, args: &[&str]) {
        let args_str = args.join(", ");
        self.output.push_str(&format!("    {}\t{}\n", op, args_str));
    }
    
    fn emit_line(&mut self, line: &str) {
        self.output.push_str(&format!("{}\n", line));
    }
}
```

---

## **Parte 5: Optimizations (optimizer.rs)**

```rust
// koi-assembly/src/optimizer.rs

use crate::ir_parser::{Instruction, IRFunction};

pub struct Optimizer;

impl Optimizer {
    pub fn optimize(func: &mut IRFunction) {
        // 1. Dead Code Elimination
        Self::dead_code_elimination(func);
        
        // 2. Constant folding (simplified)
        Self::constant_folding(func);
    }
    
    fn dead_code_elimination(func: &mut IRFunction) {
        // Mark live variables (reverse dataflow)
        let mut live = std::collections::HashSet::new();
        
        // Return value is always live
        live.insert("rax".to_string());
        
        // Traverse blocks in reverse
        for block in func.blocks.iter_mut().rev() {
            let mut to_remove = vec![];
            
            for (i, instr) in block.instructions.iter().enumerate().rev() {
                match instr {
                    Instruction::Const { result, .. } => {
                        if !live.contains(result) {
                            to_remove.push(i);
                        }
                    }
                    Instruction::BinOp { result, lhs, rhs, .. } => {
                        if !live.contains(result) {
                            to_remove.push(i);
                        } else {
                            live.insert(lhs.clone());
                            live.insert(rhs.clone());
                        }
                    }
                    _ => {}
                }
            }
            
            // Remove dead instructions
            for i in to_remove.iter().rev() {
                block.instructions.remove(*i);
            }
        }
    }
    
    fn constant_folding(_func: &mut IRFunction) {
        // Simplified: would evaluate constant expressions at compile time
    }
}
```

---

## **Main Entry Point (main.rs)**

```rust
// koi-assembly/src/main.rs

mod ir_parser;
mod register_allocator;
mod codegen;
mod optimizer;
mod abi;

use std::fs;
use ir_parser::IRParser;
use register_allocator::LinearScanAllocator;
use codegen::X86Generator;
use optimizer::Optimizer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read IR JSON from Koi-IR
    let ir_json = fs::read_to_string("/tmp/ir.json")?;
    
    println!("✓ koi-assembly starting...");
    
    // Parse IR
    let mut program = IRParser::parse_json(&ir_json)?;
    
    println!("✓ IR parsed ({} functions)", program.functions.len());
    
    // Optimize
    for func in &mut program.functions {
        Optimizer::optimize(func);
    }
    
    // Register allocation
    let mut _allocator = LinearScanAllocator::new();
    for func in &program.functions {
        let _allocation = _allocator.allocate(func);
    }
    
    // Generate x86-64 assembly
    let mut generator = X86Generator::new();
    let asm = generator.generate(&program);
    
    // Write output
    fs::write("output.s", &asm)?;
    
    println!("✓ Codegen complete. Assembly saved to output.s");
    
    Ok(())
}
```

---

## **Cargo.toml**

```toml
[package]
name = "koi-assembly"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

---

## **System V AMD64 ABI Reference**

**Argument passing:**
- First 6 integer args: `rdi`, `rsi`, `rdx`, `rcx`, `r8`, `r9`
- Additional args: stack (16-byte aligned)

**Return value:**
- `rax` (or `rdx:rax` for 128-bit)

**Caller-saved (volatile):** rax, rcx, rdx, rsi, rdi, r8-r11
**Callee-saved (non-volatile):** rbx, r12-r15, rbp, rsp

**Stack frame:**
```asm
push %rbp
mov %rsp, %rbp
sub $N, %rsp    ; N = local variables
[function body]
mov %rbp, %rsp
pop %rbp
ret
```

---

## **Example Output (output.s)**

```asm
.data
print_fmt: .string "%ld\n"

.text
.globl main

main:
    push %rbp
    mov %rsp, %rbp
    sub $16, %rsp
    
    mov $5, %rax
    mov %rax, -8(%rbp)
    
    mov $3, %rax
    mov %rax, -16(%rbp)
    
    mov -8(%rbp), %rax
    add -16(%rbp), %rax
    
    mov %rax, %rdi
    lea print_fmt(%rip), %rsi
    call printf
    
    xor %rax, %rax
    
.end_main:
    leave
    ret

.section .note.GNU-stack,"",@progbits
```

---

## **Checklist Koi-Assembly Rust (5 días)**

- [ ] Día 3: IR parser + ABI
- [ ] Día 4: Register allocator (linear scan)
- [ ] Día 5: x86-64 code generation
- [ ] Día 6: Optimizations + output
- [ ] Día 7: output.s válido, ensamblable

¡Listos para construir koi-assembly en Rust! 🦀
