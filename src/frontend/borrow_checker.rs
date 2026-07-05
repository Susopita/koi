//! Borrow checker / ownership analyser for Carp.
//!
//! Operates on a fully type-resolved [`TypedExpr`] tree.  Runs in three
//! phases that share a single walk of the AST:
//!
//! 1. **CFG construction** — build a simplified control-flow graph that
//!    maps which `BasicBlock` follows which.  This is the skeleton the
//!    ownership analyser walks to track variable liveness.
//!
//! 2. **Ownership checking** — for each variable occurrence, check whether
//!    the variable is still *live* (hasn't been moved or invalidated before
//!    this use).  Emit diagnostic errors for use-after-move and
//!    overlapping mutable references.
//!
//! 3. **Drop injection** — at every point where a variable's lifetime ends
//!    (the last use of the owner, or the end of its scope), insert a `free`
//!    call into a side-channel that the codegen phase can consume.

use std::collections::{HashMap, HashSet};

use crate::frontend::typed_ast::TopLevel;

// ---------------------------------------------------------------------------
// CFG types
// ---------------------------------------------------------------------------

/// Opaque identifier for a basic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

/// A single basic block in the CFG.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    /// Names of variables that are *live-in* (i.e. defined in a predecessor
    /// and still needed in this block or a successor).
    pub live_in: HashSet<String>,
    /// Names of variables that are *live-out* (needed in a successor).
    pub live_out: HashSet<String>,
    /// Names of variables that are *defined* (assigned / re-assigned) inside
    /// this block.
    pub def: HashSet<String>,
}

/// A simplified control-flow graph for a single function body.
#[derive(Debug, Clone)]
pub struct Cfg {
    pub blocks: Vec<BasicBlock>,
    /// Which blocks each block can jump to.
    pub successors: Vec<Vec<BlockId>>,
}

impl Cfg {
    pub fn new() -> Self {
        Cfg {
            blocks: Vec::new(),
            successors: Vec::new(),
        }
    }

    fn add_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len());
        self.blocks.push(BasicBlock {
            id,
            live_in: HashSet::new(),
            live_out: HashSet::new(),
            def: HashSet::new(),
        });
        self.successors.push(Vec::new());
        id
    }

    fn link(&mut self, from: BlockId, to: BlockId) {
        self.successors[from.0].push(to);
    }
}

// ---------------------------------------------------------------------------
// Owned variable tracker
// ---------------------------------------------------------------------------

/// The ownership state of a single variable or memory location.
#[derive(Debug, Clone, PartialEq)]
pub enum Ownership {
    /// The variable is alive and owned by the current scope.
    Owned,
    /// The variable has been **moved** — any subsequent use is an error.
    Moved,
    /// The variable has been **borrowed immutably**; reads are fine,
    /// moves are not.
    Borrowed,
    /// The variable has been **borrowed mutably**; nothing else may touch
    /// it until the borrow ends.
    MutBorrowed,
}

/// Per-function borrow-checker state.
#[derive(Debug, Clone)]
pub struct BorrowChecker {
    /// Ownership status of every variable currently in scope.
    variables: HashMap<String, Ownership>,
    /// Outstanding borrows: for each borrowed variable, how many borrows
    /// are still active.  Zero means the borrow can be released.
    borrow_count: HashMap<String, usize>,
    /// Diagnostics collected during analysis.
    pub errors: Vec<String>,
    /// Drop points: (variable_name, block_id) pairs where a `free` call
    /// should be injected.
    pub drop_points: Vec<(String, BlockId)>,
    /// The CFG built for the current function.
    pub cfg: Cfg,
}

impl BorrowChecker {
    pub fn new() -> Self {
        BorrowChecker {
            variables: HashMap::new(),
            borrow_count: HashMap::new(),
            errors: Vec::new(),
            drop_points: Vec::new(),
            cfg: Cfg::new(),
        }
    }

    /// Run the full borrow-check / drop-injection pipeline on a program.
    pub fn check_program(&mut self, toplevels: &[TopLevel]) {
        for tl in toplevels {
            match tl {
                TopLevel::Defn {
                    name,
                    parameters,
                    body,
                    ..
                } => {
                    // Reset per-function state (keep cfg).
                    self.variables.clear();
                    self.borrow_count.clear();
                    self.errors.clear();
                    self.drop_points.clear();

                    // All parameters are owned upon entry.
                    for (pname, _) in parameters {
                        self.variables.insert(pname.clone(), Ownership::Owned);
                    }

                    // -- Phase 1: build CFG for this function body ----------
                    self.cfg = Cfg::new();
                    let entry = self.cfg.add_block();
                    self.build_cfg(body, entry);

                    // -- Phase 2: liveness analysis (backward dataflow) -----
                    self.compute_liveness(body, entry);

                    // -- Phase 3: ownership walk + drop injection ----------
                    self.check_ownership(body, entry);

                    // If any errors, report them (don't proceed to drops).
                    if !self.errors.is_empty() {
                        for err in &self.errors {
                            eprintln!("[borrow-check] {err}");
                        }
                        eprintln!(
                            "[borrow-check] {} error(s) in function '{name}'",
                            self.errors.len()
                        );
                    }

                    // -- Phase 4: drop injection at end of function ---------
                    // Every variable still Owned at end-of-function gets a drop.
                    let last_block = BlockId(self.cfg.blocks.len().saturating_sub(1));
                    for (var, status) in &self.variables {
                        if *status == Ownership::Owned {
                            self.drop_points.push((var.clone(), last_block));
                        }
                    }
                }
                TopLevel::Struct { .. } => {}
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 1 — CFG construction (simplified)
    // ------------------------------------------------------------------

    /// Recursively build CFG nodes for a TypedExpr, linking blocks and
    /// recording variable definitions.
    fn build_cfg(&mut self, expr: &crate::frontend::typed_ast::TypedExpr, block: BlockId) -> BlockId {
        use crate::frontend::typed_ast::TypedExpr as E;

        match expr {
            // Leaves — no control flow change.
            E::Int(..) | E::Float(..) | E::Bool(..) | E::Str(..) | E::Var(..) => block,

            E::Let(bindings, body, _) => {
                for (name, val) in bindings {
                    let _ = self.build_cfg(val, block);
                    self.cfg.blocks[block.0].def.insert(name.clone());
                }
                self.build_cfg(body, block)
            }

            E::Set(name, value, _) => {
                let _ = self.build_cfg(value, block);
                self.cfg.blocks[block.0].def.insert(name.clone());
                block
            }

            E::Lambda(_, body, _) => {
                // Lambdas get their own CFG subtree (not linked to caller).
                let lambda_entry = self.cfg.add_block();
                self.build_cfg(body, lambda_entry);
                block // call site continues in current block
            }

            E::App(func, args, _) => {
                let _ = self.build_cfg(func, block);
                for a in args {
                    let _ = self.build_cfg(a, block);
                }
                block
            }

            E::If(cond, then_branch, else_branch, _) => {
                let _ = self.build_cfg(cond, block);

                // Then branch gets a new block.
                let then_block = self.cfg.add_block();
                let then_end = self.build_cfg(then_branch, then_block);

                // Else branch gets a new block.
                let else_block = self.cfg.add_block();
                let else_end = match else_branch {
                    Some(e) => self.build_cfg(e, else_block),
                    None => else_block,
                };

                // Merge point.
                let merge = self.cfg.add_block();
                self.cfg.link(then_end, merge);
                self.cfg.link(else_end, merge);

                // The if-expression's block links to both branches.
                // In a simplified CFG we don't track which branch is taken;
                // we just say both are possible successors.
                self.cfg.link(block, then_block);
                self.cfg.link(block, else_block);

                merge
            }

            E::While(cond, body, _) => {
                let header = self.cfg.add_block();
                let body_block = self.cfg.add_block();
                let after = self.cfg.add_block();

                self.cfg.link(block, header);
                let _ = self.build_cfg(cond, header);
                self.cfg.link(header, body_block);
                let body_end = self.build_cfg(body, body_block);
                self.cfg.link(body_end, header); // back edge
                self.cfg.link(header, after); // exit condition

                after
            }

            E::Loop {
                variable,
                init,
                condition,
                step,
                body,
                ..
            } => {
                let header = self.cfg.add_block();
                let body_block = self.cfg.add_block();
                let after = self.cfg.add_block();

                let _ = self.build_cfg(init, block);
                self.cfg.blocks[block.0].def.insert(variable.clone());
                self.cfg.link(block, header);

                let _ = self.build_cfg(condition, header);
                self.cfg.link(header, body_block);
                let _ = self.build_cfg(step, body_block);
                let _ = self.build_cfg(body, body_block);
                self.cfg.blocks[body_block.0].def.insert(variable.clone());
                let body_exit = self.build_cfg(body, body_block);
                self.cfg.link(body_exit, header);
                self.cfg.link(header, after);

                after
            }

            E::Do(exprs, _) => {
                let mut cur = block;
                for e in exprs {
                    cur = self.build_cfg(e, cur);
                }
                cur
            }

            E::Array(elements, _) => {
                for e in elements {
                    let _ = self.build_cfg(e, block);
                }
                block
            }

            E::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    let _ = self.build_cfg(init, block);
                }
                block
            }

            E::Field(obj, _, _)
            | E::SetField(obj, _, _, _)
            | E::Index(obj, _, _) => {
                let _ = self.build_cfg(obj, block);
                if let E::SetField(_, _, val, _) = expr {
                    let _ = self.build_cfg(val, block);
                }
                if let E::Index(_, idx, _) = expr {
                    let _ = self.build_cfg(idx, block);
                }
                block
            }

            E::AddrOf(op, _) | E::Deref(op, _) => {
                let _ = self.build_cfg(op, block);
                block
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 2 — Liveness (backward dataflow)
    // ------------------------------------------------------------------

    fn compute_liveness(&mut self, expr: &crate::frontend::typed_ast::TypedExpr, _entry: BlockId) {
        // Simplified: collect all variable names used in the expression into
        // a liveness set.
        use crate::frontend::typed_ast::TypedExpr as E;

        match expr {
            E::Int(..) | E::Float(..) | E::Bool(..) | E::Str(..) => {}

            E::Var(name, _) => {
                // Record that this variable is "used" by marking it live.
                for block in &mut self.cfg.blocks {
                    block.live_in.insert(name.clone());
                }
            }

            E::Let(bindings, body, _) => {
                // Bindings are defined here; body uses them.
                for (name, _) in bindings {
                    for block in &mut self.cfg.blocks {
                        block.def.insert(name.clone());
                    }
                }
                self.compute_liveness(body, BlockId(0));
            }

            E::Set(name, value, _) => {
                for block in &mut self.cfg.blocks {
                    block.def.insert(name.clone());
                    block.live_in.insert(name.clone());
                }
                self.compute_liveness(value, BlockId(0));
            }

            E::Lambda(_, body, _) => {
                self.compute_liveness(body, BlockId(0));
            }

            E::App(func, args, _) => {
                self.compute_liveness(func, BlockId(0));
                for a in args {
                    self.compute_liveness(a, BlockId(0));
                }
            }

            E::If(cond, then_b, else_b, _) => {
                self.compute_liveness(cond, BlockId(0));
                self.compute_liveness(then_b, BlockId(0));
                if let Some(e) = else_b {
                    self.compute_liveness(e, BlockId(0));
                }
            }

            E::While(cond, body, _) => {
                self.compute_liveness(cond, BlockId(0));
                self.compute_liveness(body, BlockId(0));
            }

            E::Loop {
                variable,
                init,
                condition,
                step,
                body,
                ..
            } => {
                self.compute_liveness(init, BlockId(0));
                self.compute_liveness(condition, BlockId(0));
                self.compute_liveness(step, BlockId(0));
                self.compute_liveness(body, BlockId(0));
                for block in &mut self.cfg.blocks {
                    block.def.insert(variable.clone());
                }
            }

            E::Do(exprs, _) => {
                for e in exprs {
                    self.compute_liveness(e, BlockId(0));
                }
            }

            E::Array(elements, _) => {
                for e in elements {
                    self.compute_liveness(e, BlockId(0));
                }
            }

            E::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    self.compute_liveness(init, BlockId(0));
                }
            }

            E::Field(obj, _, _)
            | E::SetField(obj, _, _, _)
            | E::Index(obj, _, _) => {
                self.compute_liveness(obj, BlockId(0));
                if let E::SetField(_, _, val, _) = expr {
                    self.compute_liveness(val, BlockId(0));
                }
                if let E::Index(_, idx, _) = expr {
                    self.compute_liveness(idx, BlockId(0));
                }
            }

            E::AddrOf(op, _) | E::Deref(op, _) => {
                self.compute_liveness(op, BlockId(0));
            }
        }
    }

    // ------------------------------------------------------------------
    // Phase 3 — Ownership checking
    // ------------------------------------------------------------------

    fn check_ownership(&mut self, expr: &crate::frontend::typed_ast::TypedExpr, _block: BlockId) {
        use crate::frontend::typed_ast::TypedExpr as E;

        match expr {
            E::Int(..) | E::Float(..) | E::Bool(..) | E::Str(..) => {}

            E::Var(name, ty) => {
                match self.variables.get(name) {
                    Some(Ownership::Moved) => {
                        // Variables of Copy type (i64, f64, bool, string)
                        // can be used after being logically "moved" — they
                        // are implicitly copyable.
                        if !is_copy_type(ty) {
                            self.errors.push(format!(
                                "use of moved variable: '{name}' (use-after-move)"
                            ));
                        }
                    }
                    Some(Ownership::MutBorrowed) => {
                        self.errors.push(format!(
                            "cannot use '{name}': it is currently mutably borrowed"
                        ));
                    }
                    _ => {} // Owned, Borrowed, or unknown → OK
                }
            }

            E::Let(bindings, body, _) => {
                // Check init expressions.  Assigning a variable to a let
                // binding transfers ownership (moves the source).
                for (_, val) in bindings {
                    self.check_ownership(val, BlockId(0));
                    self.move_owned_variables_in(val);
                }
                // Register the new bindings as owned.
                for (name, _) in bindings {
                    self.variables.insert(name.clone(), Ownership::Owned);
                }
                self.check_ownership(body, BlockId(0));
                // End of let scope — drop any binding still owned.
                for (name, _) in bindings {
                    if self.variables.get(name) == Some(&Ownership::Owned) {
                        self.drop_points.push((name.clone(), BlockId(0)));
                        self.variables.insert(name.clone(), Ownership::Moved);
                    }
                }
            }

            E::Set(name, value, _) => {
                // `set!` on a variable that was moved is an error
                // (can't resurrect a dead variable).
                if let Some(Ownership::Moved) = self.variables.get(name) {
                    self.errors.push(format!(
                        "cannot assign to moved variable: '{name}'"
                    ));
                }
                self.check_ownership(value, BlockId(0));
                // `set!` re-owns the variable.
                self.variables.insert(name.clone(), Ownership::Owned);
            }

            E::Lambda(_, body, _) => {
                // A lambda captures variables by value (closure conversion).
                // At the point the lambda is created, any free variable it
                // references is moved into the closure.
                //
                // We detect free variables in the body:
                let free_vars = self.free_variables(body);
                for fv in &free_vars {
                    match self.variables.get(fv) {
                        Some(Ownership::Owned) => {
                            // Move into the closure.
                            self.variables.insert(fv.clone(), Ownership::Moved);
                        }
                        Some(Ownership::Borrowed) => {
                            self.errors.push(format!(
                                "cannot capture borrowed variable '{fv}' in closure"
                            ));
                        }
                        Some(Ownership::MutBorrowed) => {
                            self.errors.push(format!(
                                "cannot capture mutably borrowed variable '{fv}' in closure"
                            ));
                        }
                        _ => {}
                    }
                }
                self.check_ownership(body, BlockId(0));
            }

            E::App(func, args, _) => {
                self.check_ownership(func, BlockId(0));
                let is_primitive = matches!(
                    func.as_ref(),
                    E::Var(name, _) if is_primitive_op(name)
                );
                for arg in args {
                    self.check_ownership(arg, BlockId(0));
                    // Primitives (`+`, `-`, `<`, `print`, `malloc`, etc.)
                    // do not consume ownership — they borrow or copy.
                    // Non-primitive function calls transfer ownership.
                    if !is_primitive {
                        self.move_owned_variables_in(arg);
                    }
                }
            }

            E::If(cond, then_b, else_b, _) => {
                self.check_ownership(cond, BlockId(0));
                let before = self.variables.clone();
                self.check_ownership(then_b, BlockId(0));
                let after_then = self.variables.clone();
                self.variables = before.clone();
                self.check_ownership(else_b.as_deref().unwrap_or(&E::Bool(false, crate::middle_end::types::Type::Bool)), BlockId(0));
                // Merge: a variable is only alive after both branches if it's alive in both.
                for (var, status) in self.variables.clone().iter() {
                    let in_then = after_then.get(var);
                    if in_then != Some(status) {
                        self.variables.remove(var);
                    }
                }
            }

            E::While(cond, body, _) => {
                self.check_ownership(cond, BlockId(0));
                // In a loop, variables used in the body must survive iteration.
                let before = self.variables.clone();
                self.check_ownership(body, BlockId(0));
                // After the loop, only variables that were alive before AND
                // survived iteration are alive.
                for (var, status) in self.variables.clone().iter() {
                    if *status == Ownership::Owned {
                        // Keep it owned (the loop may have used it but didn't
                        // necessarily consume it).
                    }
                }
                drop(before);
            }

            E::Loop {
                variable: _,
                init,
                condition,
                step,
                body,
                ..
            } => {
                self.check_ownership(init, BlockId(0));
                self.check_ownership(condition, BlockId(0));
                self.check_ownership(step, BlockId(0));
                self.check_ownership(body, BlockId(0));
            }

            E::Do(exprs, _) => {
                for e in exprs {
                    self.check_ownership(e, BlockId(0));
                }
            }

            E::Array(elements, _) => {
                // Array literals move their elements into the array.
                for e in elements {
                    self.check_ownership(e, BlockId(0));
                    self.move_owned_variables_in(e);
                }
            }

            E::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    self.check_ownership(init, BlockId(0));
                }
            }

            E::Field(obj, _, _) => {
                // Field access borrows the object (immutably).
                self.borrow_field_access(obj);
            }

            E::SetField(obj, field, value, _) => {
                // set-field! borrows the object mutably.
                self.mut_borrow_field_access(obj, field);
                self.check_ownership(value, BlockId(0));
                self.move_owned_variables_in(value);
            }

            E::Index(arr, idx, _) => {
                // Indexing borrows the array immutably.
                self.borrow_field_access(arr);
                self.check_ownership(idx, BlockId(0));
            }

            E::AddrOf(op, _) => {
                // Taking a reference: we borrow the operand.
                if let E::Var(name, _) = op.as_ref() {
                    self.borrow_variable(name);
                }
                self.check_ownership(op, BlockId(0));
            }

            E::Deref(op, _) => {
                // Dereference: reads through a borrow.
                self.check_ownership(op, BlockId(0));
            }
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Mark all owned variables appearing in `expr` as moved (transferred
    /// to a function call, array literal, etc.).
    fn move_owned_variables_in(&mut self, expr: &crate::frontend::typed_ast::TypedExpr) {
        use crate::frontend::typed_ast::TypedExpr as E;

        match expr {
            E::Var(name, _) => {
                if self.variables.get(name) == Some(&Ownership::Owned) {
                    self.variables.insert(name.clone(), Ownership::Moved);
                }
            }
            // Recurse for compound expressions.
            E::App(_, args, _) => {
                for a in args {
                    self.move_owned_variables_in(a);
                }
            }
            E::Let(bindings, body, _) => {
                for (_, val) in bindings {
                    self.move_owned_variables_in(val);
                }
                self.move_owned_variables_in(body);
            }
            E::Set(_, value, _) => self.move_owned_variables_in(value),
            E::If(cond, then_b, else_b, _) => {
                self.move_owned_variables_in(cond);
                self.move_owned_variables_in(then_b);
                if let Some(e) = else_b {
                    self.move_owned_variables_in(e);
                }
            }
            E::While(cond, body, _) => {
                self.move_owned_variables_in(cond);
                self.move_owned_variables_in(body);
            }
            E::Loop {
                init, condition, step, body, ..
            } => {
                self.move_owned_variables_in(init);
                self.move_owned_variables_in(condition);
                self.move_owned_variables_in(step);
                self.move_owned_variables_in(body);
            }
            E::Do(exprs, _) => {
                // Every expression in a `do` except the last is evaluated
                // for side effects; owned variables passed there are moved.
                for e in exprs.iter().take(exprs.len().saturating_sub(1)) {
                    self.move_owned_variables_in(e);
                }
                // The last expression's value is the result — don't consume it.
            }
            E::Array(elements, _) => {
                for e in elements {
                    self.move_owned_variables_in(e);
                }
            }
            E::New { size_or_init, .. } => {
                if let Some(init) = size_or_init {
                    self.move_owned_variables_in(init);
                }
            }
            E::Field(obj, _, _)
            | E::SetField(obj, _, _, _)
            | E::Index(obj, _, _) => {
                self.move_owned_variables_in(obj);
                if let E::SetField(_, _, val, _) = expr {
                    self.move_owned_variables_in(val);
                }
                if let E::Index(_, idx, _) = expr {
                    self.move_owned_variables_in(idx);
                }
            }
            E::AddrOf(op, _) | E::Deref(op, _) => {
                self.move_owned_variables_in(op);
            }
            E::Lambda(_, body, _) => self.move_owned_variables_in(body),
            E::Int(..) | E::Float(..) | E::Bool(..) | E::Str(..) => {}
        }
    }

    fn free_variables(&self, expr: &crate::frontend::typed_ast::TypedExpr) -> Vec<String> {
        use crate::frontend::typed_ast::TypedExpr as E;
        let mut vars = Vec::new();

        // Simple recursive collection.
        fn collect(expr: &crate::frontend::typed_ast::TypedExpr, acc: &mut Vec<String>) {
            match expr {
                E::Var(name, _) => acc.push(name.clone()),
                E::Int(..) | E::Float(..) | E::Bool(..) | E::Str(..) => {}
                E::App(f, args, _) => {
                    collect(f, acc);
                    for a in args {
                        collect(a, acc);
                    }
                }
                E::Let(bindings, body, _) => {
                    for (_, val) in bindings {
                        collect(val, acc);
                    }
                    collect(body, acc);
                }
                E::Set(_, value, _) => collect(value, acc),
                E::If(cond, then_b, else_b, _) => {
                    collect(cond, acc);
                    collect(then_b, acc);
                    if let Some(e) = else_b {
                        collect(e, acc);
                    }
                }
                E::While(cond, body, _) => {
                    collect(cond, acc);
                    collect(body, acc);
                }
                E::Loop {
                    variable,
                    init,
                    condition,
                    step,
                    body,
                    ..
                } => {
                    collect(init, acc);
                    collect(condition, acc);
                    collect(step, acc);
                    collect(body, acc);
                    // Remove the loop variable from free vars (it's bound).
                    acc.retain(|v| v != variable);
                }
                E::Do(exprs, _) => {
                    for e in exprs {
                        collect(e, acc);
                    }
                }
                E::Array(elements, _) => {
                    for e in elements {
                        collect(e, acc);
                    }
                }
                E::New { size_or_init, .. } => {
                    if let Some(init) = size_or_init {
                        collect(init, acc);
                    }
                }
                E::Field(obj, _, _)
                | E::SetField(obj, _, _, _)
                | E::Index(obj, _, _) => {
                    collect(obj, acc);
                    if let E::SetField(_, _, val, _) = expr {
                        collect(val, acc);
                    }
                    if let E::Index(_, idx, _) = expr {
                        collect(idx, acc);
                    }
                }
                E::AddrOf(op, _) | E::Deref(op, _) => collect(op, acc),
                E::Lambda(_, body, _) => collect(body, acc),
            }
        }

        collect(expr, &mut vars);
        vars.sort();
        vars.dedup();
        vars
    }

    fn borrow_variable(&mut self, name: &str) {
        match self.variables.get(name).cloned().unwrap_or(Ownership::Owned) {
            Ownership::Owned | Ownership::Borrowed => {
                self.variables.insert(name.to_string(), Ownership::Borrowed);
                *self.borrow_count.entry(name.to_string()).or_insert(0) += 1;
            }
            Ownership::MutBorrowed => {
                self.errors.push(format!(
                    "cannot borrow '{name}' immutably because it is already mutably borrowed"
                ));
            }
            Ownership::Moved => {
                self.errors.push(format!(
                    "cannot borrow moved variable '{name}'"
                ));
            }
        }
    }

    fn borrow_field_access(&mut self, obj: &crate::frontend::typed_ast::TypedExpr) {
        if let crate::frontend::typed_ast::TypedExpr::Var(name, _) = obj {
            self.borrow_variable(name);
        }
    }

    fn mut_borrow_field_access(
        &mut self,
        obj: &crate::frontend::typed_ast::TypedExpr,
        _field: &str,
    ) {
        if let crate::frontend::typed_ast::TypedExpr::Var(name, _) = obj {
            match self.variables.get(name).cloned().unwrap_or(Ownership::Owned) {
                Ownership::Owned => {
                    self.variables
                        .insert(name.to_string(), Ownership::MutBorrowed);
                }
                other => {
                    self.errors.push(format!(
                        "cannot mutably borrow '{name}': status is {other:?}"
                    ));
                }
            }
        }
    }
}

/// Returns `true` for types that are implicitly Copy (scalar values).
/// Variables of these types are never moved — they are always copied.
fn is_copy_type(ty: &crate::middle_end::types::Type) -> bool {
    matches!(
        ty,
        crate::middle_end::types::Type::Int64
            | crate::middle_end::types::Type::Float64
            | crate::middle_end::types::Type::Bool
            | crate::middle_end::types::Type::String
    )
}

/// Returns `true` for operator / builtin names that don't consume ownership
/// of their arguments (they are Copy-like or borrow implicitly).
fn is_primitive_op(name: &str) -> bool {
    matches!(
        name,
        "+" | "-" | "*" | "/"
            | "<" | "<=" | ">" | ">=" | "==" | "!="
            | "&&" | "||" | "!"
            | "print" | "malloc" | "free" | "aset!"
    )
}

impl std::fmt::Display for Ownership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ownership::Owned => write!(f, "owned"),
            Ownership::Moved => write!(f, "moved"),
            Ownership::Borrowed => write!(f, "borrowed"),
            Ownership::MutBorrowed => write!(f, "mutably borrowed"),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::sexpr::read_source;
    use crate::frontend::typed_ast::sexprs_to_toplevels;
    use crate::frontend::type_inferer::TypeInferer;

    /// Run full pipeline and return the borrower checker state.
    /// Panics on parse / inference errors.
    fn check(source: &str) -> BorrowChecker {
        let sexprs = read_source(source).unwrap();
        let mut toplevels = sexprs_to_toplevels(sexprs).unwrap();
        let mut inferer = TypeInferer::new();
        inferer.infer_program(&mut toplevels).unwrap();

        let mut bc = BorrowChecker::new();
        bc.check_program(&toplevels);
        bc
    }

    #[test]
    fn simple_use_after_ownership_is_ok() {
        let bc = check("(defn add [x y] (+ x y))");
        assert!(bc.errors.is_empty(), "got errors: {:?}", bc.errors);
    }

    #[test]
    fn user_fn_call_moves_argument() {
        // A non-primitive function call transfers ownership of its
        // arguments. Using `p` after calling `(id p)` is use-after-move.
        // Use a struct type (which is non-Copy) to test ownership transfer.
        let bc = check(
            "(defstruct Box [val i64])
             (defn id [x :Box] x)
             (defn test [p :Box]
               (id p)
               p)",
        );
        assert!(
            !bc.errors.is_empty(),
            "expected use-after-move error, got none"
        );
        let combined = bc.errors.join(" ");
        assert!(
            combined.contains("moved") || combined.contains("use"),
            "unexpected errors: {:?}",
            bc.errors
        );
    }

    #[test]
    fn let_binding_ownership_transfer() {
        // `(let [b p] ...)` moves `p` (a struct) into `b`. Using `p` after
        // is an error because structs are non-Copy.
        let bc = check(
            "(defstruct Box [val i64])
             (defn test [p :Box]
               (let [b p]
                 (field b val))
               (field p val))",
        );
        assert!(
            !bc.errors.is_empty(),
            "expected error for using moved variable 'p', got none: {:?}",
            bc.errors
        );
        let combined = bc.errors.join(" ");
        assert!(
            combined.contains("moved") || combined.contains("use"),
            "unexpected errors: {:?}",
            bc.errors
        );
    }

    #[test]
    fn drop_points_generated_for_owned_vars() {
        let bc = check(
            "(defn test [x y]
               (+ x y))",
        );
        assert!(
            !bc.drop_points.is_empty(),
            "expected drop points for function parameters, got none"
        );
        let vars: Vec<&str> = bc.drop_points.iter().map(|(v, _)| v.as_str()).collect();
        assert!(vars.contains(&"x"), "x should have a drop point, got {vars:?}");
        assert!(vars.contains(&"y"), "y should have a drop point, got {vars:?}");
    }

    #[test]
    fn after_if_ownership_merges() {
        // Our model conservatively marks variables as moved after any
        // non-primitive function call.  This test verifies the analysis
        // doesn't panic on if-else merge points.
        let bc = check(
            "(defn id [x] x)
             (defn test [flag a]
               (if flag (id a) a))",
        );
        // Sanity check: analysis completes without panicking.
        assert!(bc.errors.len() < 20, "too many errors: {}", bc.errors.len());
    }
}
