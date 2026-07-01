# **✅ REESCRITURA COMPLETA: Prompts C++ → Rust + Python**

**Fecha:** 29 de Junio, 2026  
**Estado:** ✅ COMPLETADO  
**Documentos generados:** 8 nuevos (en Rust)  
**Antiguo formato:** C++17 (9 documentos)  
**Nuevo formato:** Rust + Python

---

## **Resumen de Cambios**

### **De C++ a Rust:**

| Aspecto | C++ | Rust |
|---------|-----|------|
| **Lenguaje compilador** | C++17 | Latest stable |
| **AST Pattern** | Visitor pattern | Enums + pattern matching |
| **Memoria** | shared_ptr<> | Box<T>, Rc<T> |
| **JSON** | nlohmann/json | serde_json |
| **Symbol tables** | std::unordered_map | HashMap |
| **Error handling** | try/catch | Result<T, E> + ? |
| **Build system** | CMakeLists.txt | Cargo workspace |
| **Serialización** | Manual | Serde automatic |

### **De nada a Python:**

| Componente | Nuevo |
|-----------|-------|
| **IDE** | Python Textual TUI (Fase 2) |
| **Compilador wrapper** | subprocess calls |
| **Syntax highlighting** | Pygments |
| **Build/Run UI** | Buttons + output log |

---

## **Documentos Generados (8 totales)**

### **1. ✅ 00_START_HERE.md** (7 KB)
- **Propósito:** Reading guide
- **Contenido:** Quick start, roadmap, FAQ
- **Audience:** Todos (primero)
- **Tiempo lectura:** 8 min

### **2. ✅ PROMPT_MAESTRO_RUST.md** (10 KB)
- **Propósito:** Master context
- **Contenido:** 
  - Workspace Cargo structure
  - Language features
  - Pipeline architecture
  - Interface contracts (JSON schemas)
  - 7-day timeline
- **Audience:** Todos (segundo)
- **Tiempo lectura:** 15 min

### **3. ✅ PROMPT_A_RUST_AST.md** (29 KB)
- **Propósito:** Lexer + Parser + AST + Scope
- **Contenido:**
  - Token enum (serde tagged)
  - Scanner implementation
  - Recursive descent parser
  - AST enums con serde serialization
  - Scope analyzer
  - main.rs with file I/O
- **Salida:** `/tmp/ast.json`
- **Persona:** A only
- **Tiempo implementación:** 4 días

### **4. ✅ PROMPT_B_RUST_IR.md** (25 KB)
- **Propósito:** HM inference + Monomorphization + Lambda lifting + IR
- **Contenido:**
  - Type system (Type enum + Substitution)
  - Constraint generation
  - Robinson unification
  - Monomorphization with name mangling
  - Lambda lifting (closure conversion)
  - IR structures (HIR/LIR)
  - IR generator
  - main.rs JSON I/O
- **Entrada:** `/tmp/ast.json`
- **Salida:** `/tmp/ir.json`
- **Persona:** B only
- **Tiempo implementación:** 5 días

### **5. ✅ PROMPT_C_RUST_ASSEMBLY.md** (21 KB)
- **Propósito:** x86-64 codegen + Register allocation + Optimizations
- **Contenido:**
  - IR parser (JSON deserialization)
  - System V AMD64 ABI
  - Linear scan register allocator
  - x86-64 AT&T code generator
  - Optimizations (DCE, constant folding)
  - Preamble/postamble generation
  - Assembly output
- **Entrada:** `/tmp/ir.json`
- **Salida:** `output.s`
- **Persona:** C only
- **Tiempo implementación:** 5 días

### **6. ✅ INTEGRATION_RUST.md** (12 KB)
- **Propósito:** Daily sync + Cargo workspace + Testing
- **Contenido:**
  - Workspace Cargo root + per-crate configs
  - build.sh, full-compile.sh scripts
  - Interface contracts (critical)
  - Testing gates (Día 3, 5, 6)
  - Daily standup protocol
  - Error handling conventions (JSON errors)
  - Git workflow
  - Debugging tips
  - Contingency plan
- **Audience:** Todos (durante semana)
- **Frecuencia:** Daily reference

### **7. ✅ QUICK_REF_RUST.md** (11 KB)
- **Propósito:** Rust patterns cheat sheet
- **Contenido:** 
  - 20 Rust idioms (copy-paste patterns)
  - Enum + pattern matching
  - Result<T, E> error handling
  - HashMap, Vec, Option
  - String vs &str
  - Box<T> for recursion
  - Serde JSON
  - Closures, lifetimes
  - File I/O
  - DO's/DON'Ts
  - Debugging tips
  - Common errors & fixes
- **Audience:** Todos (bookmark)
- **Uso:** Durante coding

### **8. ✅ PROMPT_IDE_PYTHON.md** (17 KB)
- **Propósito:** Python Textual IDE (Fase 2, bonus)
- **Contenido:**
  - Architecture (IDE → subprocess → Rust compiler)
  - compiler.py (subprocess wrapper, full pipeline)
  - syntax_highlighter.py (Pygments)
  - ui.py (Textual components)
  - main.py (App entry)
- **Tecnología:** Python 3.10+, Textual, Pygments
- **Timeline:** Semana 2 (si MVP listo)
- **Audience:** Team (optional)

---

## **Cambios Clave vs C++ Version**

### **En la Arquitectura:**

```diff
- C++: visitor pattern (TypeChecker, GenCode visitors)
+ Rust: enums + match (idiomatic, no virtual dispatch)

- C++: shared_ptr<Exp> recursion
+ Rust: Box<ASTNode> (cheaper, no atomic refcount)

- C++: nlohmann/json manual mapping
+ Rust: serde_json automatic derive(Serialize, Deserialize)

- C++: std::unordered_map + Environment stack
+ Rust: HashMap + Vec<HashMap> (same semantics, better Rust)

- C++: try/catch exception handling
+ Rust: Result<T, E> with ? operator (zero-cost, explicit)

- C++: CMakeLists.txt single executable
+ Rust: Cargo workspace (3 separate binaries, cleaner separation)
```

### **En File I/O:**

```diff
- C++: std::ifstream, std::ofstream
+ Rust: std::fs::read_to_string(), std::fs::write()

- C++: iostream buffering
+ Rust: direct file I/O (Rust handles buffering)
```

### **En Testing:**

```diff
- C++: googletest or custom
+ Rust: cargo test (built-in)
```

---

## **Contenido Reutilizado de C++**

✅ **Conservado del original (C++):**
- Language feature list (HM inference, closures, generics, etc.)
- Pipeline architecture (3 phases: frontend, typesystem, backend)
- JSON interface contracts (same schemas, now with serde)
- x86-64 AT&T output format
- System V AMD64 ABI details
- 7-day MVP timeline
- Testing gates (Día 3, 5, 6)

❌ **Descartado (C++ specific):**
- CMakeLists.txt syntax
- shared_ptr/unique_ptr patterns
- visitor pattern boilerplate
- nlohmann/json mapping code
- C++ iostream patterns

🆕 **Agregado (Rust specific):**
- Enum + pattern matching examples
- Serde derive macros
- Result<T, E> error handling
- Cargo workspace configuration
- Rust idiom cheat sheet (20 patterns)
- Python IDE (Fase 2)

---

## **Verificación: Todos los Archivos**

### **Nuevos (Rust):**
✅ `00_START_HERE.md` (7 KB)
✅ `PROMPT_MAESTRO_RUST.md` (10 KB)
✅ `PROMPT_A_RUST_AST.md` (29 KB)
✅ `PROMPT_B_RUST_IR.md` (25 KB)
✅ `PROMPT_C_RUST_ASSEMBLY.md` (21 KB)
✅ `INTEGRATION_RUST.md` (12 KB)
✅ `QUICK_REF_RUST.md` (11 KB)
✅ `PROMPT_IDE_PYTHON.md` (17 KB)

**Total:** 8 archivos, 132 KB, ~2000 líneas de contenido

### **Antiguos (C++, conservados en repo):**
- PROMPT_MAESTRO_CARP_COMPILADOR.md (C++)
- PROMPT_A_AST.md (C++)
- PROMPT_B_IR.md (C++)
- PROMPT_C_ASSEMBLY.md (C++)
- INTEGRATION_GUIDE.md (C++)
- QUICK_REFERENCE.md (C++)
- README_PROMPTS.md (C++)
- INDEX_PROMPTS.md (C++)

**Total:** 8 archivos (legacy), para referencia

---

## **Cómo Usar Estos Prompts**

### **Día 1:**
1. Todos leen `00_START_HERE.md` (8 min)
2. Todos leen `PROMPT_MAESTRO_RUST.md` (15 min)
3. Cada persona lee su track específico (30-40 min)
4. Setup Cargo workspace (20 min)
5. **Total: ~2 horas**

### **Semana 1 (MVP):**
- A: Implementa AST (4 días)
- B: Implementa IR (5 días, espera A)
- C: Implementa Assembly (5 días, espera B)
- Todos: Daily standup (5 min each)
- Todos: Usan `QUICK_REF_RUST.md` como bookmark

### **Semana 2 (Bonus):**
- Si MVP completo: Implementar IDE Python (Fase 2)
- Benchmarking vs GCC
- Final polish + submission

---

## **Especificaciones Técnicas Incluidas**

✅ **Type System:**
- Hindley-Milner inference algorithm
- Robinson unification
- Type variable management
- Substitution composition

✅ **Closures:**
- Lambda lifting algorithm
- Free variable analysis
- Environment struct generation
- Fat pointer creation

✅ **Generics:**
- Monomorphization strategy
- Name mangling scheme
- Specialization algorithm

✅ **Code Generation:**
- System V AMD64 ABI
- Linear scan register allocation
- x86-64 AT&T syntax
- Dead code elimination
- Constant folding
- Peephole optimization

✅ **Testing:**
- Gate 1 (Día 3): AST JSON
- Gate 2 (Día 5): IR JSON
- Gate 3 (Día 6): Assembly ensamblable
- Integration tests

---

## **Diferencias Críticas: C++ vs Rust**

### **1. AST Representation**

**C++:**
```cpp
class Exp { virtual ~Exp() = default; };
class BinaryExp : public Exp { /* ... */ };
// Visitor pattern needed for operations
```

**Rust:**
```rust
enum ASTNode {
    BinaryOp { /* ... */ },
}
// Direct pattern matching, no virtual dispatch
```

**Ventaja Rust:** Safer, faster, more idiomatic.

---

### **2. JSON Serialization**

**C++:**
```cpp
json obj;
obj["name"] = "test";
obj["value"] = 42;
// Manual field mapping
```

**Rust:**
```rust
#[derive(Serialize)]
struct Data { name: String, value: i64 }
// Automatic via serde
```

**Ventaja Rust:** Zero-copy, automatic, macro-driven.

---

### **3. Error Handling**

**C++:**
```cpp
try {
    parser.parse();
} catch (std::exception& e) {
    // Handle error
}
```

**Rust:**
```rust
parser.parse()?  // Early return on error
// Or: match parser.parse() { Ok => {...}, Err => {...} }
```

**Ventaja Rust:** Explicit, zero-cost, no unwinding overhead.

---

### **4. Workspace**

**C++:**
```
CMakeLists.txt (single monolithic build)
src/
  all sources mixed
```

**Rust:**
```
Cargo.toml (workspace root)
koi-ast/ (binary 1)
koi-ir/ (binary 2)
koi-assembly/ (binary 3)
```

**Ventaja Rust:** Clear separation, independent compilation.

---

## **Próximos Pasos para el Team**

1. **Read:** Comienzan con `00_START_HERE.md`
2. **Setup:** Clone Lab10, setup Cargo workspace
3. **Code:** Sigan estructura en los prompts
4. **Sync:** Daily 5-min standups
5. **Test:** Validar gates en días 3, 5, 6
6. **Submit:** Día 7 MVP completo
7. **Bonus:** Semana 2 IDE Python (opcional)

---

## **Recursos incluidos en los Prompts**

✅ 8 documentos completos
✅ 80+ ejemplos de código Rust
✅ Cargo workspace configuration
✅ Build scripts (bash)
✅ JSON schema definitions
✅ Testing protocols
✅ Debugging tips
✅ 20 Rust idiom patterns
✅ Python IDE starter code
✅ Daily sync protocol
✅ Contingency plans

---

## **Conclusión**

**De C++ a Rust:** Reescritura completa, manteniendo arquitectura probada.

**Cambios clave:**
- ✅ Enums + pattern matching (idiomatic Rust)
- ✅ Serde JSON (automatic serialization)
- ✅ Result<T, E> (zero-cost errors)
- ✅ Cargo workspace (3 separate binaries)
- ✅ Python IDE (bonus Fase 2)

**Ready for:**
- MVP in Week 1 ✅
- IDE in Week 2 ✅
- Benchmarks & Polish ✅

---

**¡Koi está listo para ser construido!** 🦀

Comenzar ahora con `00_START_HERE.md`.
