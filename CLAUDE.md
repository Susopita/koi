# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

**Koi** is a compiler for a Lisp-like language called **Carp**, targeting x86-64 assembly, written in Rust. The repo is currently at the scaffolding stage: the Cargo workspace and three crates exist, but each crate's `src/main.rs` is still the default `Hello, world!` template and no `Cargo.toml` has its `serde`/`serde_json`/`regex` dependencies wired up yet. The bulk of the repository is design documentation (`PROMPT_*.md`, `INTEGRATION_RUST.md`, `QUICK_REF_RUST.md`) written as specs for a 3-person, 7-day implementation sprint — treat these as the source of truth for what still needs to be built.

Read `00_START_HERE.md` for the doc reading order. The most important docs for implementation work are:
- `PROMPT_MAESTRO_RUST.md` — overall architecture, JSON interface contracts, language feature list.
- `PROMPT_A_RUST_AST.md` — lexer/parser/AST/scope spec (koi-ast).
- `PROMPT_B_RUST_IR.md` — Hindley-Milner inference, monomorphization, lambda lifting, IR spec (koi-ir).
- `PROMPT_C_RUST_ASSEMBLY.md` — register allocation, x86-64 codegen, optimizations spec (koi-assembly).
- `INTEGRATION_RUST.md` — build scripts, testing gates, error format, git workflow.
- `QUICK_REF_RUST.md` — Rust idiom cheat sheet (enum/match AST style, serde patterns, Result-based errors).

## Commands

```bash
# Build all crates (release)
./build.sh
# or directly:
cargo build --release

# Build/test a single crate
cargo build -p koi-ast --release
cargo test -p koi-ir --release

# Type-check without building
cargo check

# Format / lint
cargo fmt
cargo clippy
```

There is no `full-compile.sh` in the repo yet (only documented in `INTEGRATION_RUST.md`); once the pipeline is implemented, the intended end-to-end flow is:

```bash
./target/release/koi-ast test-programs/add.carp   # -> /tmp/ast.json
./target/release/koi-ir                            # reads /tmp/ast.json -> /tmp/ir.json
./target/release/koi-assembly                       # reads /tmp/ir.json -> output.s
gcc -c output.s -o output.o && gcc output.o -o output
./output
```

There is no `test-programs/` directory yet — sample `.carp` files referenced in the docs (`add.carp`, `fib.carp`, `lambda.carp`, `struct.carp`) need to be created.

## Architecture

The compiler is a **3-stage pipeline**, implemented as three independent Rust binaries in one Cargo workspace, communicating via JSON files in `/tmp` rather than in-process calls:

```
test.carp
  -> koi-ast       (lexer + recursive-descent parser + AST + scope analysis)
       writes /tmp/ast.json
  -> koi-ir        (Hindley-Milner inference + monomorphization + lambda lifting + IR gen)
       reads /tmp/ast.json, writes /tmp/ir.json
  -> koi-assembly  (IR parsing + linear-scan register allocation + x86-64 AT&T codegen + peephole opts)
       reads /tmp/ir.json, writes output.s
  -> gcc assembles/links output.s into an executable
```

Each crate is meant to grow the internal module layout described in `INTEGRATION_RUST.md`:
- `koi-ast/src/`: `token.rs`, `scanner.rs`, `parser.rs`, `ast.rs`, `scope.rs`
- `koi-ir/src/`: `types.rs`, `inference.rs`, `unification.rs`, `monomorphizer.rs`, `lambda_lifter.rs`, `ir.rs`, `ir_generator.rs`
- `koi-assembly/src/`: `ir_parser.rs`, `register_allocator.rs`, `codegen.rs`, `optimizer.rs`, `abi.rs`

### Interface contracts (must stay in sync across crates)

The three binaries are decoupled and only agree via JSON shapes — changing one without updating the consuming crate breaks the pipeline silently until run. Full schemas are in `PROMPT_MAESTRO_RUST.md`; summary:

- **AST JSON** (koi-ast -> koi-ir): tagged enum on `nodeType` (`program`, `function_def`, `call`, `literal`, ...), every node carries `line`/`column` for diagnostics.
- **IR JSON** (koi-ir -> koi-assembly): `{ irType: "hir"|"lir", functions: [...] }`, instructions are a tagged enum on `op` (`const`, `binop`, `return`, ...) using SSA-style `%vN` value names.
- **Error format** (all binaries, written to stderr): `{ phase, severity, message, location: { file, line, column } }`, followed by `std::process::exit(1)`.

When implementing any stage, prefer Rust enums + `#[serde(tag = "...")]` + pattern matching over a visitor pattern — this is the idiom the design docs standardize on (see `QUICK_REF_RUST.md`).

### Carp language (MVP scope)

S-expression syntax; `i64`/`f64`/`bool`/`string` primitives (everything 64-bit); `defn` functions with recursion; `defstruct` user types; 1D/2D arrays; explicit pointers (`&`, `*`); `malloc`/`free`; lambdas/closures via closure conversion; generics via monomorphization; full Hindley-Milner inference; `if`/`let`/`loop` control flow.

### Workspace quirks to be aware of

- The root `Cargo.toml` sets `edition = "2026"` under `[workspace.package]`, but each crate's own `Cargo.toml` currently hardcodes `edition = "2024"` directly instead of inheriting via `edition.workspace = true` — the per-crate values are what `cargo build` actually uses.
- `serde`, `serde_json`, and `regex` are declared under `[workspace.dependencies]` but no crate's `[dependencies]` references them yet (`{ workspace = true }`) — this needs to be added before any JSON (de)serialization code will compile.
- Phase 2 (optional bonus, not required for the MVP grade) is a Python Textual TUI IDE (`pond/`) that wraps the three Rust binaries via `subprocess`; spec lives in `PROMPT_IDE_PYTHON.md` and hasn't been started.
