# **🦀 PROMPT MAESTRO: Koi, Compilador Carp en Rust → x86-64**

## **Contexto General del Proyecto**

Implementar un **compilador completo Koi (Carp en Rust)** → **x86-64** en **Rust** (fase 1), con un **IDE en Python Textual** (fase 2). Arquitectura de 3 tracks paralelos + workspace Cargo.

**Stack Tecnológico:**
- **Compilador:** Rust (latest stable)
- **Serialización:** serde_json
- **AST:** Enums + pattern matching (idiomático Rust)
- **IDE:** Python Textual (Fase 2)
- **IPC:** JSON files (/tmp) + subprocess

---

## **Características del Lenguaje (MVP - Hard Requirements)**

✅ S-expressions básicas (sin macros homoicónicas)
✅ Tipos básicos (i64, f64, bool, strings) — **TODO es 64-bit**
✅ Funciones (defn), parámetros, recursión
✅ Tipos definidos por usuario (defstruct)
✅ Arreglos 1D y 2D
✅ Punteros explícitos (&, *)
✅ Memoria dinámica (malloc/free)
✅ Lambdas y closures (closure conversion)
✅ Genéricos/templates (monomorphization)
✅ Inferencia de tipos Hindley-Milner
✅ Control de flujo (if/let/loop)

---

## **Arquitectura: 2 Workspaces, 3 Crates en Koi + 1 en Python**

```
koi/ (Workspace compiler)
├── Cargo.toml (workspace config)
│
├── koi-ast/          (Persona A)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         (entry point)
│       ├── token.rs
│       ├── scanner.rs
│       ├── parser.rs
│       ├── ast.rs
│       └── scope.rs
│
├── koi-ir/        (Persona B)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs         (reads /tmp/ast.json)
│       ├── types.rs
│       ├── inference.rs
│       ├── unification.rs
│       ├── monomorphizer.rs
│       ├── lambda_lifter.rs
│       └── ir.rs
│
└─── koi-assembly/           (Persona C)
    ├── Cargo.toml
    └── src/
        ├── main.rs         (reads /tmp/ir.json)
        ├── ir_parser.rs
        ├── register_allocator.rs
        ├── codegen.rs
        └── optimizer.rs

pond/               (Fase 2 - IDE CLI en Python)
├── main.py
└── ...
```

**Workspace Cargo.toml:**
```toml
[workspace]
members = ["koi-ast", "koi-ir", "koi-assembly"]
resolver = "2"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

---

## **Contratos de Interfaz (Críticos)**

### **INTERFACE 1: AST JSON (A → B)**

```json
{
  "nodeType": "program|function_def|call|variable|literal|...",
  "name": "optional_name",
  "line": 1,
  "column": 1,
  "children": [],
  "typeAnnotation": null
}
```

**En Rust (Persona A serializará con serde):**
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "nodeType")]
pub enum ASTNode {
    #[serde(rename = "program")]
    Program { children: Vec<ASTNode> },
    
    #[serde(rename = "function_def")]
    FunctionDef {
        name: String,
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "call")]
    Call {
        function: Box<ASTNode>,
        arguments: Vec<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "literal")]
    Literal {
        #[serde(rename = "literalType")]
        literal_type: String, // "int64", "float64", "bool", "string"
        value: serde_json::Value,
        line: usize,
        column: usize,
    },
    
    // ... más variantes
}
```

---

### **INTERFACE 2: Typed IR JSON (B → C)**

```json
{
  "irType": "hir",
  "functions": [{
    "name": "main",
    "returnType": "i64",
    "parameters": [...],
    "blocks": [{
      "label": "entry",
      "instructions": [{
        "op": "const",
        "result": "%v0",
        "type": "i64"
      }]
    }]
  }]
}
```

**En Rust (Persona B serializará con serde):**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IRProgram {
    #[serde(rename = "irType")]
    pub ir_type: String, // "hir" o "lir"
    pub functions: Vec<IRFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IRFunction {
    pub name: String,
    #[serde(rename = "returnType")]
    pub return_type: String,
    pub parameters: Vec<(String, String)>,
    pub blocks: Vec<BasicBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Instruction {
    #[serde(rename = "const")]
    Const {
        result: String,
        value: serde_json::Value,
        #[serde(rename = "type")]
        ty: String,
    },
    #[serde(rename = "binop")]
    BinOp {
        result: String,
        lhs: String,
        rhs: String,
        #[serde(rename = "op_type")]
        op_type: String,
        #[serde(rename = "type")]
        ty: String,
    },
    // ... más variantes
}
```

---

### **INTERFACE 3: Error Format Unificado**

```json
{
  "phase": "lexer|parser|semantic|codegen",
  "severity": "error|warning",
  "message": "Clear description",
  "location": {
    "file": "test.carp",
    "line": 42,
    "column": 5
  }
}
```

**En Rust:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileError {
    pub phase: String,
    pub severity: String,
    pub message: String,
    pub location: ErrorLocation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
}
```

---

## **Pipeline de Compilación**

```
test.carp
    ↓
[Persona A: koi-ast binary]
  Scanner (lexer.rs)
  Parser (parser.rs)
  AST (ast.rs)
  Scope (scope.rs)
    ↓ Output: /tmp/ast.json
    
[Persona B: koi-ir binary]
  Read /tmp/ast.json
  Hindley-Milner inference
  Monomorphization
  Lambda lifting
  IR generation (HIR → LIR)
    ↓ Output: /tmp/ir.json
    
[Persona C: koi-assembly binary]
  Read /tmp/ir.json
  Register allocation
  x86-64 codegen
  Optimizations
    ↓ Output: output.s
    
[Sistema]
  gcc -c output.s -o output.o
  gcc output.o -o executable
  ./executable
```

---

## **Configuración Cargo Workspace**

**koi/Cargo.toml:**
```toml
[workspace]
members = ["koi-ast", "koi-ir", "koi-assembly"]
resolver = "2"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
regex = "1.10"
```

**Cada crate tiene su Cargo.toml:**

**koi-ast/Cargo.toml:**
```toml
[package]
name = "koi-ast"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
regex = { workspace = true }

[[bin]]
name = "koi-ast"
path = "src/main.rs"
```

**koi-ir/Cargo.toml:**
```toml
[package]
name = "koi-ir"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }

[[bin]]
name = "koi-ir"
path = "src/main.rs"
```

**koi-assembly/Cargo.toml:**
```toml
[package]
name = "koi-assembly"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }

[[bin]]
name = "koi-assembly"
path = "src/main.rs"
```

---

## **Construcción del Compilador Completo**

**Makefile o script build.sh:**
```bash
#!/bin/bash
set -e

echo "=== Building Koi ==="

# Build all crates
cargo build --release

# Test
cargo test --release

echo "✓ Build complete"
echo ""
echo "Binaries at:"
echo "  ./target/release/koi-ast"
echo "  ./target/release/koi-ir"
echo "  ./target/release/koi-assembly"
```

**Full compilation example:**
```bash
./target/release/koi-ast test.carp  # → /tmp/ast.json
./target/release/koi-ir           # → /tmp/ir.json
./target/release/koi-assembly       # → output.s
gcc -c output.s -o output.o
gcc output.o -o output
./output
```

---

## **Características de Rust Aprovechadas**

✅ **Enums + pattern matching** en lugar de visitor pattern (más idiomático)
✅ **Result<T, E>** para error handling
✅ **match** para control flow
✅ **serde** para JSON automático
✅ **String ownership** para parseo seguro
✅ **Vec<T>** para colecciones
✅ **HashMap/BTreeMap** para symbol tables
✅ **Lifetimes** para references eficientes
✅ **No null pointers** (Optional<T> en su lugar)

---

## **Timeline de 1 Semana (Rust)**

| Día | Persona A | Persona B | Persona C |
|-----|-----------|-----------|-----------|
| 1-2 | Scanner + Parser enums | Tipos base | x86 templates |
| 3 | AST serde, scope | HM inference | Register allocation |
| 4 | Full parser + tests | Monomorph | Full codegen |
| 5 | Polish + scope | Lambda lifting + IR | Optimizations |
| 6 | Integration | IR JSON final | Benchmarks |
| 7 | — | — | Final polish |

---

## **MVP Checklist**

- [ ] Workspace Cargo creado
- [ ] Lexer con pattern matching en Rust
- [ ] Parser recursive descent con Result<T>
- [ ] AST con serde JSON
- [ ] Scope analysis
- [ ] HM type inference
- [ ] Monomorphization
- [ ] Lambda lifting
- [ ] IR generation
- [ ] Register allocation
- [ ] x86-64 codegen
- [ ] Optimizations
- [ ] Todos tests pasando
- [ ] Benchmarks vs GCC

---

## **IDE Python (Fase 2 - Próximos Prompts)**

```
pond/
├── main.py
├── ui.py
├── syntax_highlighter.py
└── requirements.txt
```

**Tecnología:**
- Python 3.10+
- Textual (TUI framework)
- Subprocess para llamar binarios Rust
- JSON para IPC

---

## **Referencias Importantes**

- Lab9/Lab10 structure: recursive descent parser
- Rust idioms: enums, pattern matching, Result
- serde documentation: https://serde.rs/
- Cargo workspaces: https://doc.rust-lang.org/cargo/reference/workspaces.html
- System V AMD64 ABI: x86-64 calling conventions

---

## **Siguientes Pasos**

1. **Persona A:** Lee `PROMPT_A_RUST_AST.md`
2. **Persona B:** Lee `PROMPT_B_RUST_IR.md`
3. **Persona C:** Lee `PROMPT_C_RUST_ASSEMBLY.md`
4. **Todos:** Lee `INTEGRATION_RUST.md`
5. **Comienzan a codificar**

**¡Adelante con Rust!** 🦀
