# **🦀 INTEGRATION_RUST.md — Daily Sync, Cargo Workspace, Coordination**

**Objetivo:** Coordinar trabajo en paralelo de 3 personas en Rust, mantener contratos claros, integrar dailies.

**Duración:** 1 semana (7 días)

---

## **Estructura del Workspace Cargo**

```
koi/
├── Cargo.toml                 ← WORKSPACE ROOT
├── Cargo.lock                 ← Generated, commit to repo
├── build.sh                   ← Full build script
├── benchmark.sh               ← Benchmark script
│
├── koi-ast/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs
│   │   ├── token.rs
│   │   ├── scanner.rs
│   │   ├── parser.rs
│   │   ├── ast.rs
│   │   └── scope.rs
│   └── tests/
│       └── test_*.rs
│
├── koi-ir/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs
│   │   ├── types.rs
│   │   ├── inference.rs
│   │   ├── unification.rs
│   │   ├── monomorphizer.rs
│   │   ├── lambda_lifter.rs
│   │   ├── ir.rs
│   │   └── ir_generator.rs
│   └── tests/
│
├── koi-assembly/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs
│   │   ├── ir_parser.rs
│   │   ├── register_allocator.rs
│   │   ├── codegen.rs
│   │   ├── optimizer.rs
│   │   └── abi.rs
│   └── tests/
│
├── test-programs/
│   ├── fib.carp
│   ├── add.carp
│   ├── lambda.carp
│   └── struct.carp
│
└── README.md
```

---

## **Cargo Workspace Root (Cargo.toml)**

```toml
[workspace]
members = ["koi-ast", "koi-ir", "koi-assembly"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["Sopitas"]

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
regex = "1.10"

[profile.release]
opt-level = 3
lto = true
```

---

## **Build Scripts**

### **build.sh**

```bash
#!/bin/bash
set -e

RESET='\033[0m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'

echo -e "${BLUE}=== Building Koi ===${RESET}"
echo ""

cargo build --release 2>&1 | grep -E "Compiling|Finished|error|warning" || true

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Build successful${RESET}"
else
    echo "Build failed"
    exit 1
fi

echo ""
echo "Binaries at:"
echo "  ./target/release/koi-ast"
echo "  ./target/release/koi-ir"
echo "  ./target/release/koi-assembly"
```

### **full-compile.sh** (Integration test)

```bash
#!/bin/bash
set -e

INPUT_FILE="${1:-test-programs/add.carp}"

if [ ! -f "$INPUT_FILE" ]; then
    echo "Error: File not found: $INPUT_FILE"
    exit 1
fi

echo "=== Full Compilation Pipeline ==="
echo "Input: $INPUT_FILE"
echo ""

# Step 1: AST
echo "[1/4] AST (lexing + parsing)..."
./target/release/koi-ast "$INPUT_FILE"

if [ ! -f "/tmp/ast.json" ]; then
    echo "ERROR: AST generation failed"
    exit 1
fi
echo "✓ AST saved to /tmp/ast.json"
echo ""

# Step 2: IR
echo "[2/4] IR (HM inference + IR)..."
./target/release/koi-ir

if [ ! -f "/tmp/ir.json" ]; then
    echo "ERROR: IR generation failed"
    exit 1
fi
echo "✓ IR saved to /tmp/ir.json"
echo ""

# Step 3: Assembly
echo "[3/4] Assembly (code generation)..."
./target/release/koi-assembly

if [ ! -f "output.s" ]; then
    echo "ERROR: Assembly generation failed"
    exit 1
fi
echo "✓ Assembly saved to output.s"
echo ""

# Step 4: Assemble + Link
echo "[4/4] Assembling and linking..."
gcc -c output.s -o output.o
gcc output.o -o output

if [ -f "output" ]; then
    echo "✓ Executable created: output"
    echo ""
    echo "=== Output ==="
    ./output
else
    echo "ERROR: Linking failed"
    exit 1
fi
```

---

## **Contratos de Interfaz (Críticos)**

### **Contrato 1: A → B (/tmp/ast.json)**

**Responsabilidad de A:**
- Generar JSON válido según esquema
- Incluir todos `lineNumber` y `column` para errores
- Validar scope (sin variables no declaradas)
- Salida en `/tmp/ast.json` (sobreescribir cada run)

**Esperado por B:**
```json
{
  "nodeType": "program",
  "children": [
    {
      "nodeType": "function_def",
      "name": "add",
      "parameters": [["x", "i64"], ["y", "i64"]],
      "body": {...},
      "line": 1,
      "column": 1
    }
  ]
}
```

**Validar:**
```bash
cat /tmp/ast.json | jq . > /dev/null && echo "✓ Valid JSON" || echo "✗ Invalid JSON"
```

---

### **Contrato 2: B → C (/tmp/ir.json)**

**Responsabilidad de B:**
- Generar IR tipado
- HM inference correcta (no type variables sin resolver)
- Monomorphizar funciones genéricas
- Salida en `/tmp/ir.json`

**Esperado por C:**
```json
{
  "irType": "hir",
  "functions": [
    {
      "name": "add",
      "returnType": "i64",
      "parameters": [["x", "i64"], ["y", "i64"]],
      "blocks": [
        {
          "label": "entry",
          "instructions": [
            {"op": "const", "result": "%v0", "value": 5, "type": "i64"},
            {"op": "binop", "result": "%v1", "lhs": "%v0", "rhs": "%v2", "op_type": "+", "type": "i64"},
            {"op": "return", "value": "%v1"}
          ]
        }
      ]
    }
  ]
}
```

**Validar:**
```bash
cat /tmp/ir.json | jq '.functions[0].blocks[0].instructions' | head -5
```

---

### **Contrato 3: C → Executable (output.s)**

**Responsabilidad de C:**
- x86-64 AT&T válido
- Ensamblable con `gcc -c`
- Linkeable, ejecutable

**Test:**
```bash
gcc -c output.s -o output.o && echo "✓ Assembles" || echo "✗ Syntax error"
```

---

## **Timeline de 7 Días**

| Día | Persona A | Persona B | Persona C | Sync |
|-----|-----------|-----------|-----------|------|
| 1 | Lexer enum + Scanner | Type enum + Subst | x86 templates | Setup workspace |
| 2 | Parser (partial) | Constraint gen | Register allocator | Demo lexer output |
| 3 | Parser (complete) | Unification | Codegen (partial) | Demo AST JSON |
| 4 | AST + Serde + Scope | Monomorph | Codegen (complete) | Demo IR JSON |
| 5 | Test + polish | Lambda lift + IR gen | Optimizations | Integration test |
| 6 | — | Polish IR | Benchmark + refine | Full pipeline |
| 7 | — | — | Final output | **SUBMISSION** |

---

## **Daily Sync Protocol (5 min standup)**

**Cada mañana (10:00 o similar):**

1. **Persona A:** "Lexer/parser status? AST JSON ready?" → Yes/No + blockers
2. **Persona B:** "HM inference done? IR JSON structure finalized?" → Yes/No + blockers
3. **Persona C:** "Codegen x86 ready? Benchmarks working?" → Yes/No + blockers

**Formato:**
```
[NAME] Status: [COMPONENT] [%complete]
  Blockers: [none / awaiting AST from A / etc]
  Next: [task for next 24h]
```

**Ejemplo:**
```
[A] Status: Parser 75%
    Blockers: Need clarification on lambda syntax from maestro
    Next: Complete expr parser, test with simple examples
    
[B] Status: HM unification 50%
    Blockers: Waiting for final AST JSON schema (READY from A ✓)
    Next: Implement constraint generation for calls
    
[C] Status: x86 codegen 30%
    Blockers: Need IR structure from B (Waiting)
    Next: Implement basic binop code generation
```

---

## **Testing & Validation Gates**

### **Gate 1: AST (Día 3)**

**Test case: test-programs/add.carp**
```lisp
(defn add [x y]
  (+ x y))

(defn main []
  (add 5 3))
```

**Validate:**
```bash
./target/release/koi-ast test-programs/add.carp
cat /tmp/ast.json | jq '.children[0].name'  # Should output "add"
```

**Pass condition:** Valid JSON, all nodes have line/column, no scope errors

---

### **Gate 2: IR (Día 5)**

**Test case: test-programs/fib.carp**
```lisp
(defn fib [n]
  (if (<= n 1)
    n
    (+ (fib (- n 1)) (fib (- n 2)))))
```

**Validate:**
```bash
./target/release/koi-ir
cat /tmp/ir.json | jq '.functions[0].returnType'  # Should be "i64"
```

**Pass condition:** Valid IR JSON, all types resolved, correct function signatures

---

### **Gate 3: Assembly (Día 6)**

**Test case: any .carp → output.s**

**Validate:**
```bash
./target/release/koi-assembly
gcc -c output.s -o output.o  # Must succeed
gcc output.o -o output        # Must link
./output                      # Must not crash
```

**Pass condition:** Executable runs without segfault

---

## **Workspace Commands**

```bash
# Build all crates
cargo build --release

# Test all crates
cargo test --release

# Build one crate
cargo build -p koi-ast --release

# Run tests for one crate
cargo test -p koi-ir --release

# Check compilation without building
cargo check

# Format code
cargo fmt

# Lint
cargo clippy
```

---

## **Repository Structure (GitHub)**

```
koi/
├── .github/
│   └── workflows/
│       └── ci.yml               ← Auto-build on push
├── .gitignore
├── README.md
├── Cargo.toml                   ← Workspace
├── build.sh
├── full-compile.sh
├── [3 crates]
└── test-programs/
```

**.gitignore:**
```
/target/
/output.s
/output.o
/output
*.swp
*.tmp
/tmp/ast.json
/tmp/ir.json
.DS_Store
```

---

## **Error Handling Conventions**

Todos los binarios deben escribir errores a **stderr** en formato JSON:

```json
{
  "phase": "parser",
  "severity": "error",
  "message": "Expected closing paren",
  "location": {
    "file": "test.carp",
    "line": 5,
    "column": 10
  }
}
```

**En Rust:**
```rust
eprintln!("{}", serde_json::to_string_pretty(&error)?);
std::process::exit(1);
```

---

## **Git Workflow**

**Individual branches per track:**
```bash
git checkout -b ast/lexer
git checkout -b ir/inference
git checkout -b assembly/codegen
```

**Daily:**
```bash
git add src/
git commit -m "[A] Lexer complete + Scanner 90%"
git push origin ast/lexer
```

**Integration:**
```bash
# On main
git merge --no-ff ast/lexer
./full-compile.sh test-programs/add.carp  # Validate
```

---

## **Debugging Tips**

**Si A produce JSON inválido:**
```bash
cat /tmp/ast.json | jq . # Will show parse errors
```

**Si B produce IR inválido:**
```bash
cat /tmp/ir.json | jq '.functions[0].blocks[0].instructions'
```

**Si C produce assembly inválido:**
```bash
gcc -Wall -c output.s 2>&1 | head -20  # Show errors
```

---

## **Benchmarking (Día 6-7)**

**test-programs/fib.carp:**
```lisp
(defn fib [n] ...)
(defn main [] (fib 30))
```

**Benchmark:**
```bash
# Compilador Carp
time ./output

# GCC reference (same algorithm in C)
gcc -O3 -o fib_gcc test-programs/fib.c
time ./fib_gcc
```

**Expected:** Carp output debe estar dentro de 2x GCC (acceptable MVP)

---

## **Checklist Final (Día 7)**

- [ ] Workspace Cargo builds cleanly
- [ ] All 3 crates compile without warnings
- [ ] Full pipeline: carp → AST → IR → .s → executable
- [ ] 3 test programs work: add, fib, lambda
- [ ] Benchmarks vs GCC documented
- [ ] All code formatted (cargo fmt)
- [ ] No clippy warnings
- [ ] README completo
- [ ] **SUBMIT**

---

## **Contingency Plan**

Si alguien se atrasa:

**AST OK, IR slow:**
- C puede escribir IR parser + codegen dummy
- A puede ayudar a optimizar parser

**IR OK, Assembly slow:**
- A y B pueden escribir assembly generator tests
- Usar IR generators pre-escrito

**Assembly slow, tiempo agotado:**
- Usar output.s pre-escrito para benchmark
- Focus en pipeline integration

¡Adelante con Rust! 🦀
