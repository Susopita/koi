# **📚 00_START_HERE.md — Reading Order & Document Index**

**Bienvenido a Koi, el compilador de Carp implementado en Rust.**

Esto es la guía para leer los 8 documentos en orden correcto.

---

## **Quick Start (5 min read)**

1. **Este archivo** (00_START_HERE.md) ← You are here
2. **PROMPT_MAESTRO_RUST.md** (15 min) — Contexto general
3. **Tu track personal:**
   - **Persona A:** PROMPT_A_RUST_AST.md
   - **Persona B:** PROMPT_B_RUST_IR.md
   - **Persona C:** PROMPT_C_RUST_ASSEMBLY.md
4. **INTEGRATION_RUST.md** (todos) — Daily sync
5. **QUICK_REF_RUST.md** (todos) — Keep open while coding

**Timeline:** Comienza hoy, semana 1 = MVP, semana 2 = IDE bonus.

---

## **El Proyecto en 30 Segundos**

```
Compilador Koi (Rust)
└─ S-expressions
└─ Tipos: int64, float64, bool, string, arrays, structs, ptrs
└─ Características: HM type inference, closures, generics, monomorphization
└─ Output: x86-64 AT&T assembly (ensamblable con gcc)

3 Personas, 3 Tracks Paralelos:
├─ A: AST (Lexer + Parser → AST JSON)
├─ B: IR (HM + IR → IR JSON)
└─ C: ASSEMBLY (Codegen + Assembly → output.s)

IDE Bonus:
└─ Python Textual TUI (Fase 2, después de MVP)
```

---

## **Documento por Documento**

### **1. PROMPT_MAESTRO_RUST.md** (Para todos)
**Leer PRIMERO después de este.**

- Contexto completo del proyecto
- Workspace Cargo estructura
- Contratos de interfaz (A→B→C JSON)
- Características del lenguaje Carp
- Pipeline de compilación
- Timeline de 7 días

**¿Por qué primero?** Define los términos, estructura, y cómo todo conecta.

---

### **2. PROMPT_A_RUST_AST.md** (Solo Persona A)

**Responsabilidad:** Lexer + Parser + AST + Scope

**Contenido:**
- Token enum (Rust idiomático)
- Scanner: character-by-character tokenization
- Parser: recursive descent LL(1)
- AST: enums con serde JSON serialization
- Scope analyzer: variable declarations

**Salida:** `/tmp/ast.json`

**Timeline:** Días 1-4

**Comenzar:** Después de leer MAESTRO

---

### **3. PROMPT_B_RUST_IR.md** (Solo Persona B)

**Responsabilidad:** Hindley-Milner inference + Monomorphization + Lambda lifting + IR

**Contenido:**
- Type system: Type enum + Substitution + Robinson unification
- Constraint generation
- Unification algorithm
- Monomorphization (name mangling)
- Lambda lifting (closure conversion)
- IR structures (HIR → LIR)
- IR generator

**Entrada:** `/tmp/ast.json` (de Persona A)
**Salida:** `/tmp/ir.json`

**Timeline:** Días 2-6

**Comenzar:** Después de leer MAESTRO + ver PROMPT_A (para AST structure)

---

### **4. PROMPT_C_RUST_ASSEMBLY.md** (Solo Persona C)

**Responsabilidad:** x86-64 codegen + Register allocation + Optimizations

**Contenido:**
- IR parser (lee `/tmp/ir.json`)
- System V AMD64 ABI
- Linear scan register allocator
- x86-64 AT&T code generator
- Dead code elimination + constant folding
- Assembly output

**Entrada:** `/tmp/ir.json` (de Persona B)
**Salida:** `output.s` (x86-64 assembly)

**Timeline:** Días 3-7

**Comenzar:** Después de leer MAESTRO + ver PROMPT_B (para IR structure)

---

### **5. INTEGRATION_RUST.md** (Para todos, dailies)

**Responsabilidad:** Coordinación y sincronización

**Contenido:**
- Workspace Cargo detallado
- Build scripts (build.sh, full-compile.sh)
- Contratos de interfaz JSON (crítico)
- Testing gates (día 3, 5, 6)
- Daily standup format
- Error handling conventions
- Git workflow
- Debugging tips
- Benchmarking

**Frecuencia:** Leer ANTES de Primera reunión de sync

**Usar como referencia:** Durante toda la semana

---

### **6. QUICK_REF_RUST.md** (Para todos, bookmark)

**Responsabilidad:** Cheat sheet de idiomas Rust

**Contenido:**
- 20 patrones Rust comunes
- Enum + pattern matching
- Result<T, E> error handling
- HashMap, Vec, Option
- String vs &str
- Box<T> for recursive types
- Serde JSON serialization
- Closures, lifetimes
- File I/O
- DO's and DON'Ts
- Debugging tips

**Cómo usar:** Imprime o mantén en pestaña durante coding.

**Ejemplo uso:**
> "¿Cómo serializo con serde?" → Abre QUICK_REF, busca "Pattern 7"

---

### **7. PROMPT_IDE_PYTHON.md** (Fase 2, después de MVP)

**Responsabilidad:** Python Textual IDE (opcional, bonus)

**Contenido:**
- Architecture IDE ↔ Compiler
- Compiler wrapper (subprocess)
- Syntax highlighter (Pygments)
- Textual UI (editor + output)
- Build & Run buttons
- Execution panel

**Tecnología:** Python 3.10+, Textual, Pygments

**Timeline:** Semana 2, si MVP está listo

**No es obligatorio para pasar.**

---

## **Workflow por Rol**

### **Si eres Persona A (Frontend)**

**Orden de lectura:**
1. Este archivo (5 min)
2. PROMPT_MAESTRO_RUST.md (15 min)
3. PROMPT_A_RUST_AST.md (30 min, profundo)
4. Vistazo a PROMPT_B (5 min) — para ver qué espera de ti
5. INTEGRATION_RUST.md (10 min)
6. QUICK_REF_RUST.md (bookmark)

**First 2 hours:**
- Setup Cargo workspace
- Crear structure de directorios
- Implementar Token enum
- Escribir primer test

---

### **Si eres Persona B (Type System)**

**Orden de lectura:**
1. Este archivo (5 min)
2. PROMPT_MAESTRO_RUST.md (15 min)
3. PROMPT_A_RUST_AST.md (10 min, vistazo) — para entender AST
4. PROMPT_B_RUST_IR.md (40 min, profundo)
5. Vistazo a PROMPT_C (5 min) — para ver qué espera de ti
6. INTEGRATION_RUST.md (10 min)
7. QUICK_REF_RUST.md (bookmark)

**First 2 hours:**
- Setup Cargo workspace
- Implementar Type enum
- Escribir Substitution struct
- Placeholder para Constraint gen

---

### **Si eres Persona C (Backend)**

**Orden de lectura:**
1. Este archivo (5 min)
2. PROMPT_MAESTRO_RUST.md (15 min)
3. PROMPT_B_RUST_IR.md (10 min, vistazo) — para entender IR
4. PROMPT_C_RUST_ASSEMBLY.md (35 min, profundo)
5. INTEGRATION_RUST.md (10 min)
6. QUICK_REF_RUST.md (bookmark)

**First 2 hours:**
- Setup Cargo workspace
- Implementar IR parser
- Implementar ABI constants
- Escribir primer test

---

## **Checklist Día 0 (Hoy)**

- [ ] Todos leyeron 00_START_HERE.md
- [ ] Todos leyeron PROMPT_MAESTRO_RUST.md
- [ ] Persona A: Leyó PROMPT_A_RUST_AST.md
- [ ] Persona B: Leyó PROMPT_B_RUST_IR.md
- [ ] Persona C: Leyó PROMPT_C_RUST_ASSEMBLY.md
- [ ] Todos bookmark QUICK_REF_RUST.md
- [ ] Todos instalaron Rust (rustup, latest stable)
- [ ] Clonaron Lab10 repo (referencia)
- [ ] Setup Cargo workspace inicial

**Tiempo total:** ~2 horas

---

## **Documentos Generados**

✅ `00_START_HERE.md` — Este archivo
✅ `PROMPT_MAESTRO_RUST.md` — Contexto general
✅ `PROMPT_A_RUST_AST.md` — Frontend track
✅ `PROMPT_B_RUST_IR.md` — Type system track
✅ `PROMPT_C_RUST_ASSEMBLY.md` — Backend track
✅ `INTEGRATION_RUST.md` — Daily sync & testing
✅ `QUICK_REF_RUST.md` — Rust patterns cheat sheet
✅ `PROMPT_IDE_PYTHON.md` — IDE Fase 2 (opcional)

**Total:** 8 documentos, ~200 KB, ~1000 líneas de contenido + código

---

## **Próximo Paso**

👉 **Abre `PROMPT_MAESTRO_RUST.md` ahora.**

---

**Tiempo leer este archivo: 8 min**

¡Adelante con Rust! 🦀
