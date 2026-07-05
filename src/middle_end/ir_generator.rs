use crate::frontend::ast::ASTNode;
use crate::middle_end::builtins::{BuiltinKind, builtin_kind};
use crate::middle_end::ir::{BasicBlock, IRFunction, IRProgram, Instruction};
use crate::middle_end::types::Type;
use std::collections::HashMap;

/// Lowers the (already-typed, already-lifted) AST to HIR instructions.
///
/// `if` and `loop` get real multi-block SSA codegen (`Branch`/`Jump`/`Phi`)
/// regardless of position -- an `if` used as an expression branches to a
/// `then`/`else` block and merges the result with a `Phi` in a join block;
/// a `loop` branches to a header block whose `Phi` carries the loop
/// variable across iterations via a back-edge. This works whether the
/// `if`/`loop` is in tail position or not, since `generate_expr` always
/// leaves its result usable in whatever block is now current -- callers
/// don't need to know if new blocks got created underneath them.
///
/// Struct/pointer/array operations and calls through a function-valued
/// parameter (e.g. `apply-func`'s `f`) get their own dedicated instructions
/// (`Alloc`/`GetField`/`GetIndex`/`AddrOf`/`Deref`/`CallIndirect`) rather
/// than being encoded as synthetic `Call`s to made-up names.
pub struct IRGenerator<'a> {
    functions: &'a HashMap<String, Type>,
    struct_fields: &'a HashMap<String, Vec<(String, Type)>>,
    temp_counter: usize,
    label_counter: usize,
    blocks: Vec<BasicBlock>,
    current_label: String,
    current_instructions: Vec<Instruction>,
    scopes: Vec<HashMap<String, (String, String)>>,
    /// Per-env-struct-type field-name -> real (post-monomorphization) type,
    /// populated as a side effect of generating a `MakeClosure` expression
    /// (see `generate_make_closure`) and consulted by `field_type` when
    /// resolving a `FieldAccess` onto one of these synthetic env structs
    /// (as opposed to a real user `defstruct`, resolved via
    /// `struct_fields`). Keyed by `format!("env_{function_name}")`.
    closure_env_types: HashMap<String, HashMap<String, String>>,
}

impl<'a> IRGenerator<'a> {
    pub fn new(
        functions: &'a HashMap<String, Type>,
        struct_fields: &'a HashMap<String, Vec<(String, Type)>>,
    ) -> Self {
        IRGenerator {
            functions,
            struct_fields,
            temp_counter: 0,
            label_counter: 0,
            blocks: vec![],
            current_label: "entry".to_string(),
            current_instructions: vec![],
            scopes: vec![],
            closure_env_types: HashMap::new(),
        }
    }

    pub fn generate_program(&mut self, program: &ASTNode) -> Result<IRProgram, String> {
        let children = match program {
            ASTNode::Program { children } => children,
            other => return Err(format!("expected top-level program, got {other:?}")),
        };

        // Two passes: every ordinary (non-lifted-lambda) function first,
        // then every lifted lambda function. A `MakeClosure` expression
        // lives inside some ordinary function's body (e.g. `main`) and
        // populates `self.closure_env_types` as a side effect of being
        // generated (see `generate_make_closure`) -- the corresponding
        // lifted lambda function's own body needs that map already
        // populated by the time *it* gets generated, since its rewritten
        // `env.field` accesses (see `lambda_lifter.rs`'s
        // `rewrite_free_var_access`) read each captured field's real type
        // out of it via `field_type`. `lambda_lifter.rs` names every lifted
        // lambda `format!("_lambda_{id}")`, so that prefix is the signal
        // used to sort a function into the second pass.
        let is_lifted_lambda = |name: &str| name.starts_with("_lambda_");

        let mut ordinary = vec![];
        let mut lifted = vec![];
        for child in children {
            if let ASTNode::FunctionDef { name, .. } = child {
                if is_lifted_lambda(name) {
                    lifted.push(child);
                } else {
                    ordinary.push(child);
                }
            }
            // StructDef: nothing to emit at the instruction level for this MVP.
        }

        let mut functions = vec![];
        for child in ordinary.into_iter().chain(lifted) {
            if let ASTNode::FunctionDef {
                name,
                parameters,
                body,
                ..
            } = child
            {
                functions.push(self.generate_function(name, parameters, body)?);
            }
        }

        Ok(IRProgram {
            ir_type: "hir".to_string(),
            functions,
        })
    }

    fn generate_function(
        &mut self,
        name: &str,
        parameters: &[(String, Option<String>)],
        body: &ASTNode,
    ) -> Result<IRFunction, String> {
        self.temp_counter = 0;
        self.label_counter = 0;
        self.blocks.clear();
        self.current_label = "entry".to_string();
        self.current_instructions.clear();
        self.scopes.clear();
        self.scopes.push(HashMap::new());

        let (param_types, return_type) = match self.functions.get(name) {
            Some(Type::Function {
                params,
                return_type,
            }) => (params.clone(), (**return_type).clone()),
            _ => (vec![Type::Int64; parameters.len()], Type::Int64),
        };

        let mut ir_params = vec![];
        for ((pname, _), pty) in parameters.iter().zip(param_types.iter()) {
            let ty_str = pty.mangled_name();
            // Parameters are already-materialized values at function
            // entry: their own name doubles as their "temp".
            self.declare(pname, pname.clone(), ty_str.clone());
            ir_params.push((pname.clone(), ty_str));
        }

        let (result, _) = self.generate_expr(body)?;
        self.push_instruction(Instruction::Return {
            value: Some(result),
        });
        self.finish_block();

        Ok(IRFunction {
            name: name.to_string(),
            return_type: return_type.mangled_name(),
            parameters: ir_params,
            blocks: std::mem::take(&mut self.blocks),
        })
    }

    fn generate_expr(&mut self, node: &ASTNode) -> Result<(String, String), String> {
        match node {
            ASTNode::Literal {
                literal_type,
                value,
                ..
            } => {
                let ty = literal_ir_type(literal_type);
                let result = self.new_temp();
                self.push_instruction(Instruction::Const {
                    result: result.clone(),
                    value: value.clone(),
                    ty: ty.clone(),
                });
                Ok((result, ty))
            }

            ASTNode::Variable { name, line, column } => {
                if let Some(binding) = self.lookup(name) {
                    return Ok(binding);
                }
                if let Some(ty) = self.functions.get(name) {
                    // A bare reference to a top-level/lifted function name
                    // (what a zero-capture lifted lambda becomes). There's
                    // no first-class function value in this instruction
                    // set, so the name itself stands in as a placeholder
                    // (a real backend would need a function-pointer here).
                    let return_ty = match ty {
                        Type::Function { return_type, .. } => return_type.mangled_name(),
                        other => other.mangled_name(),
                    };
                    return Ok((name.clone(), return_ty));
                }
                Err(format!(
                    "Undefined variable '{name}' at line {line}, column {column}"
                ))
            }

            ASTNode::Call {
                function,
                arguments,
                ..
            } => self.generate_call(function, arguments),

            ASTNode::LetBinding { bindings, body, .. } => {
                self.scopes.push(HashMap::new());
                for (name, value) in bindings {
                    let (temp, ty) = self.generate_expr(value)?;
                    self.declare(name, temp, ty);
                }
                let result = self.generate_expr(body);
                self.scopes.pop();
                result
            }

            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.generate_if(condition, then_branch, else_branch.as_deref()),

            ASTNode::LoopExpr {
                variable,
                init,
                condition,
                step,
                body,
                ..
            } => self.generate_loop(variable, init, condition, step, body),

            ASTNode::Lambda { .. } => {
                Err("lambdas must be lifted before IR generation".to_string())
            }

            ASTNode::MakeClosure {
                function_name,
                captured,
                ..
            } => self.generate_make_closure(function_name, captured),

            ASTNode::FieldAccess { object, field, .. } => {
                let (obj_temp, obj_ty) = self.generate_expr(object)?;
                let ty = self.field_type(&obj_ty, field);
                let result = self.new_temp();
                self.push_instruction(Instruction::GetField {
                    result: result.clone(),
                    object: obj_temp,
                    field: field.clone(),
                    ty: ty.clone(),
                });
                Ok((result, ty))
            }

            ASTNode::SetField {
                object,
                field,
                value,
                ..
            } => {
                let (obj_temp, obj_ty) = self.generate_expr(object)?;
                let (value_temp, _) = self.generate_expr(value)?;
                let ty = self.field_type(&obj_ty, field);
                self.push_instruction(Instruction::SetField {
                    object: obj_temp,
                    field: field.clone(),
                    value: value_temp,
                    ty,
                });
                Ok(self.unit_result())
            }

            ASTNode::Index { array, index, .. } => {
                let (arr_temp, arr_ty) = self.generate_expr(array)?;
                let (idx_temp, _) = self.generate_expr(index)?;
                let elem_ty = elem_type_of(&arr_ty);
                let result = self.new_temp();
                self.push_instruction(Instruction::GetIndex {
                    result: result.clone(),
                    array: arr_temp,
                    index: idx_temp,
                    ty: elem_ty.clone(),
                });
                Ok((result, elem_ty))
            }

            ASTNode::AddrOf { operand, .. } => {
                let (temp, ty) = self.generate_expr(operand)?;
                let ptr_ty = format!("ptr_{ty}");
                let result = self.new_temp();
                self.push_instruction(Instruction::AddrOf {
                    result: result.clone(),
                    operand: temp,
                    ty: ptr_ty.clone(),
                });
                Ok((result, ptr_ty))
            }

            ASTNode::Deref { operand, .. } => {
                let (temp, ty) = self.generate_expr(operand)?;
                let pointee_ty = ty.strip_prefix("ptr_").unwrap_or("i64").to_string();
                let result = self.new_temp();
                self.push_instruction(Instruction::Deref {
                    result: result.clone(),
                    operand: temp,
                    ty: pointee_ty.clone(),
                });
                Ok((result, pointee_ty))
            }

            ASTNode::New {
                type_str,
                size_or_init,
                ..
            } => {
                let size = match size_or_init {
                    Some(init) => Some(self.generate_expr(init)?.0),
                    None => None,
                };
                let result = self.new_temp();
                self.push_instruction(Instruction::Alloc {
                    result: result.clone(),
                    ty: type_str.clone(),
                    size,
                });
                Ok((result, type_str.clone()))
            }

            ASTNode::ArrayLiteral { elements, .. } => {
                // Allocate space for the array, then write each element in
                // with `SetIndex` (the `aset!` write counterpart to
                // `GetIndex`) -- before `SetIndex` existed, this only
                // allocated zeroed/uninitialized space and silently dropped
                // every element's value on the floor.
                let mut elem_ty = "i64".to_string();
                let mut element_temps = Vec::with_capacity(elements.len());
                for element in elements {
                    let (temp, ty) = self.generate_expr(element)?;
                    elem_ty = ty;
                    element_temps.push(temp);
                }
                let arr_ty = format!("arr_{elem_ty}");
                // Every element is 8 bytes in this MVP (i64/f64/bool/string/
                // ptr_*/arr_*/fn_* all uniformly size to 8 in koi-assembly's
                // `type_size`), so the buffer needs `elements.len() * 8`
                // bytes -- passing `size: None` here previously always fell
                // back to a hardcoded 64-byte allocation regardless of how
                // many elements the literal actually had, silently
                // corrupting the heap for any literal with more than 8
                // elements.
                let size_temp = self.new_temp();
                self.push_instruction(Instruction::Const {
                    result: size_temp.clone(),
                    value: serde_json::Value::from((elements.len() * 8) as i64),
                    ty: "i64".to_string(),
                });
                let result = self.new_temp();
                self.push_instruction(Instruction::Alloc {
                    result: result.clone(),
                    ty: arr_ty.clone(),
                    size: Some(size_temp),
                });
                for (index, value_temp) in element_temps.into_iter().enumerate() {
                    let index_temp = self.new_temp();
                    self.push_instruction(Instruction::Const {
                        result: index_temp.clone(),
                        value: serde_json::Value::from(index as i64),
                        ty: "i64".to_string(),
                    });
                    self.push_instruction(Instruction::SetIndex {
                        array: result.clone(),
                        index: index_temp,
                        value: value_temp,
                        ty: elem_ty.clone(),
                    });
                }
                Ok((result, arr_ty))
            }

            ASTNode::SetVar { name, value, .. } => {
                let (value_temp, value_ty) = self.generate_expr(value)?;
                self.reassign(name, value_temp, value_ty)?;
                Ok(self.unit_result())
            }

            ASTNode::WhileExpr {
                condition, body, ..
            } => self.generate_while(condition, body),

            ASTNode::DoExpr { exprs, .. } => {
                let mut last = None;
                for expr in exprs {
                    last = Some(self.generate_expr(expr)?);
                }
                Ok(last.expect("parser guarantees non-empty do"))
            }

            ASTNode::Program { .. } | ASTNode::FunctionDef { .. } | ASTNode::StructDef { .. } => {
                Err(format!("'{node:?}' cannot appear inside an expression"))
            }
        }
    }

    /// `if` as an expression: branch, evaluate each side in its own block,
    /// then merge with a `Phi` in a join block. Works in tail and non-tail
    /// position alike -- the caller just keeps using the returned value in
    /// whatever block is current afterward.
    fn generate_if(
        &mut self,
        condition: &ASTNode,
        then_branch: &ASTNode,
        else_branch: Option<&ASTNode>,
    ) -> Result<(String, String), String> {
        let (cond_temp, _) = self.generate_expr(condition)?;
        let then_label = self.new_label("if_then");
        let else_label = self.new_label("if_else");
        let merge_label = self.new_label("if_merge");

        self.push_instruction(Instruction::Branch {
            cond: cond_temp,
            true_label: then_label.clone(),
            false_label: else_label.clone(),
        });
        self.finish_block();

        // Snapshot every visible variable before branching. Now that `set!`
        // exists, either branch may mutate a variable that's still visible
        // after the `if` -- without merging those mutations here, whichever
        // branch's SSA temp happened to be generated last would "win"
        // regardless of which branch actually runs at runtime, and reading
        // a temp defined in only one branch from the merge point onward is
        // an SSA violation (garbage/uninitialized reads when the other
        // branch is the one actually taken).
        let pre_if_snapshot = self.snapshot_visible_vars();

        self.current_label = then_label;
        let (then_value, then_ty) = self.generate_expr(then_branch)?;
        let then_end_label = self.current_label.clone();
        let post_then = self.snapshot_named(&pre_if_snapshot)?;
        self.push_instruction(Instruction::Jump {
            label: merge_label.clone(),
        });
        self.finish_block();

        // Restore the pre-if scope state before generating the else branch,
        // so it starts from the same place the then-branch did rather than
        // building on top of the then-branch's mutations.
        for (name, pre_temp, ty) in &pre_if_snapshot {
            self.reassign(name, pre_temp.clone(), ty.clone())?;
        }

        self.current_label = else_label;
        let (else_value, _) = match else_branch {
            Some(else_branch) => self.generate_expr(else_branch)?,
            None => {
                // No else branch: this project's test programs never hit
                // this (every `if` has one), so fall back to a default
                // value of the then-branch's type rather than modeling a
                // real "unit" type.
                let result = self.new_temp();
                self.push_instruction(Instruction::Const {
                    result: result.clone(),
                    value: default_value_for_type(&then_ty),
                    ty: then_ty.clone(),
                });
                (result, then_ty.clone())
            }
        };
        let else_end_label = self.current_label.clone();
        let post_else = self.snapshot_named(&pre_if_snapshot)?;
        self.push_instruction(Instruction::Jump {
            label: merge_label.clone(),
        });
        self.finish_block();

        self.current_label = merge_label;

        // Merge each variable whose value differs between the two branches
        // via a Phi; variables neither branch touched already agree (both
        // still point at their pre-if temp) and need no Phi.
        for ((name, then_temp, ty), (_, else_temp, _)) in post_then.iter().zip(post_else.iter()) {
            if then_temp != else_temp {
                let phi_result = self.new_temp();
                self.push_instruction(Instruction::Phi {
                    result: phi_result.clone(),
                    incoming: vec![
                        (then_end_label.clone(), then_temp.clone()),
                        (else_end_label.clone(), else_temp.clone()),
                    ],
                    ty: ty.clone(),
                });
                self.reassign(name, phi_result, ty.clone())?;
            }
        }

        let result = self.new_temp();
        self.push_instruction(Instruction::Phi {
            result: result.clone(),
            incoming: vec![(then_end_label, then_value), (else_end_label, else_value)],
            ty: then_ty.clone(),
        });
        Ok((result, then_ty))
    }

    /// Looks up the *current* value of every `(name, _, _)` in `names`,
    /// e.g. to compare a branch's post-execution state against a prior
    /// snapshot taken with `snapshot_visible_vars`.
    fn snapshot_named(
        &self,
        names: &[(String, String, String)],
    ) -> Result<Vec<(String, String, String)>, String> {
        names
            .iter()
            .map(|(name, _, _)| {
                let (temp, ty) = self
                    .lookup(name)
                    .ok_or_else(|| format!("internal error: '{name}' vanished from scope"))?;
                Ok((name.clone(), temp, ty))
            })
            .collect()
    }

    /// `loop` as an expression: a header block's `Phi` carries the loop
    /// variable in from either the pre-loop block or the body's back-edge;
    /// the loop's value is the variable's value at the point the condition
    /// finally fails (the only value that naturally reaches the exit block).
    fn generate_loop(
        &mut self,
        variable: &str,
        init: &ASTNode,
        condition: &ASTNode,
        step: &ASTNode,
        body: &ASTNode,
    ) -> Result<(String, String), String> {
        let (init_temp, init_ty) = self.generate_expr(init)?;
        let before_label = self.current_label.clone();

        let header_label = self.new_label("loop_header");
        let body_label = self.new_label("loop_body");
        let exit_label = self.new_label("loop_exit");

        self.push_instruction(Instruction::Jump {
            label: header_label.clone(),
        });
        self.finish_block();

        self.current_label = header_label.clone();
        let var_temp = self.new_temp();
        // The back-edge from the loop body isn't known yet; patched in below
        // once it's been generated (standard SSA loop-header construction).
        self.push_instruction(Instruction::Phi {
            result: var_temp.clone(),
            incoming: vec![(before_label, init_temp)],
            ty: init_ty.clone(),
        });
        self.scopes.push(HashMap::new());
        self.declare(variable, var_temp.clone(), init_ty.clone());

        let (cond_temp, _) = self.generate_expr(condition)?;
        self.push_instruction(Instruction::Branch {
            cond: cond_temp,
            true_label: body_label.clone(),
            false_label: exit_label.clone(),
        });
        self.finish_block();

        self.current_label = body_label;
        let _ = self.generate_expr(body)?; // evaluated for side effects; the loop's value is the variable, not the body
        let (step_temp, _) = self.generate_expr(step)?;
        let latch_label = self.current_label.clone();
        self.push_instruction(Instruction::Jump {
            label: header_label.clone(),
        });
        self.finish_block();
        self.scopes.pop();

        self.patch_loop_phi(&header_label, &var_temp, latch_label, step_temp);

        self.current_label = exit_label;
        Ok((var_temp, init_ty))
    }

    /// Adds the loop body's back-edge to the header block's `Phi`, which was
    /// already flushed to `self.blocks` before the body (and thus the
    /// back-edge value) existed.
    fn patch_loop_phi(
        &mut self,
        header_label: &str,
        phi_result: &str,
        from_label: String,
        value: String,
    ) {
        let Some(block) = self.blocks.iter_mut().find(|b| b.label == header_label) else {
            return;
        };
        for instruction in &mut block.instructions {
            if let Instruction::Phi {
                result, incoming, ..
            } = instruction
                && result == phi_result
            {
                incoming.push((from_label, value));
                return;
            }
        }
    }

    /// `while` as an expression: unlike `loop`, arbitrarily many pre-existing
    /// variables can be mutated (via `set!`) in the body, not just one
    /// declared loop variable. Every variable visible at the point the
    /// `while` starts gets its own header `Phi` carrying it across
    /// iterations, mirroring `generate_loop`'s single-variable technique but
    /// generalized to the whole visible scope. `while`'s own value (like
    /// `loop`'s) is never the body's value -- it's synthesized as `unit`.
    fn generate_while(
        &mut self,
        condition: &ASTNode,
        body: &ASTNode,
    ) -> Result<(String, String), String> {
        let snapshot = self.snapshot_visible_vars();
        let before_label = self.current_label.clone();

        let header_label = self.new_label("while_header");
        let body_label = self.new_label("while_body");
        let exit_label = self.new_label("while_exit");

        self.push_instruction(Instruction::Jump {
            label: header_label.clone(),
        });
        self.finish_block();

        self.current_label = header_label.clone();
        // One Phi per visible variable; only the pre-loop incoming edge is
        // known so far. Each Phi's result immediately replaces the
        // variable's binding so the condition/body see the Phi'd value
        // rather than the pre-loop one -- the back-edge is patched in below,
        // once the body (and thus each variable's post-body value) exists.
        let mut phis: Vec<(String, String, String)> = vec![]; // (name, phi_result, ty)
        for (name, pre_temp, ty) in &snapshot {
            let phi_result = self.new_temp();
            self.push_instruction(Instruction::Phi {
                result: phi_result.clone(),
                incoming: vec![(before_label.clone(), pre_temp.clone())],
                ty: ty.clone(),
            });
            self.reassign(name, phi_result.clone(), ty.clone())?;
            phis.push((name.clone(), phi_result, ty.clone()));
        }

        let (cond_temp, _) = self.generate_expr(condition)?;
        self.push_instruction(Instruction::Branch {
            cond: cond_temp,
            true_label: body_label.clone(),
            false_label: exit_label.clone(),
        });
        self.finish_block();

        self.current_label = body_label;
        let _ = self.generate_expr(body)?; // evaluated for side effects only, exactly like `loop`'s body
        let latch_label = self.current_label.clone();
        self.push_instruction(Instruction::Jump {
            label: header_label.clone(),
        });
        self.finish_block();

        for (name, phi_result, _ty) in &phis {
            let (post_temp, _post_ty) = self.lookup(name).ok_or_else(|| {
                format!("internal error: '{name}' vanished from scope after while body")
            })?;
            self.patch_loop_phi(&header_label, phi_result, latch_label.clone(), post_temp);
        }

        // The body's generation left each variable's scope entry pointing at
        // whatever SSA temp it last computed *inside the body* -- but that
        // temp is only defined when the loop actually ran the body one more
        // time. On exit (condition false), the correct, always-defined value
        // for each variable is its header Phi result, not the last body-only
        // temp -- restore that here so code after the loop reads the right
        // (and always-defined) value instead of one that's only live on a
        // path that wasn't necessarily taken.
        for (name, phi_result, ty) in &phis {
            self.reassign(name, phi_result.clone(), ty.clone())?;
        }

        self.current_label = exit_label;
        Ok(self.unit_result())
    }

    /// Every variable visible right now, across all scope levels -- i.e.
    /// exactly what `lookup(name)` would return for each name, for every
    /// name currently in scope. Walks innermost-to-outermost, keeping only
    /// the first (innermost) binding seen per name, same shadowing rule
    /// `lookup` already follows.
    fn snapshot_visible_vars(&self) -> Vec<(String, String, String)> {
        let mut seen = std::collections::HashSet::new();
        let mut result = vec![];
        for scope in self.scopes.iter().rev() {
            for (name, (temp, ty)) in scope {
                if seen.insert(name.clone()) {
                    result.push((name.clone(), temp.clone(), ty.clone()));
                }
            }
        }
        result
    }

    /// Synthesizes a fresh `unit`-typed value (`set!`, `while`, and `aset!`
    /// don't naturally produce one, but every expression needs to return
    /// *something* usable by the caller).
    fn unit_result(&mut self) -> (String, String) {
        let result = self.new_temp();
        self.push_instruction(Instruction::Const {
            result: result.clone(),
            value: serde_json::Value::Null,
            ty: "unit".to_string(),
        });
        (result, "unit".to_string())
    }

    /// Constructs an actual closure value for a `MakeClosure` node (emitted
    /// by `lambda_lifter.rs` for a lambda that captures free variables):
    /// allocates an env struct populated with each captured variable's
    /// *real* current type (known here, post-monomorphization -- unlike at
    /// lifting time, where only names were known), then wraps it with the
    /// lifted function's pointer in the shared two-field `Closure` struct.
    /// Returns a `"closure_{function_name}"`-typed value; `generate_call`'s
    /// indirect-call path recognizes that prefix as "this callee needs
    /// env/fn-ptr unpacking before the call," as opposed to a bare function
    /// pointer.
    fn generate_make_closure(
        &mut self,
        function_name: &str,
        captured: &[String],
    ) -> Result<(String, String), String> {
        let env_struct_name = format!("env_{function_name}");

        let env_temp = self.new_temp();
        self.push_instruction(Instruction::Alloc {
            result: env_temp.clone(),
            ty: env_struct_name.clone(),
            size: None,
        });

        let mut field_types = HashMap::new();
        for name in captured {
            let (value_temp, ty) = self.lookup(name).ok_or_else(|| {
                format!("internal error: captured variable '{name}' not found in scope")
            })?;
            self.push_instruction(Instruction::SetField {
                object: env_temp.clone(),
                field: name.clone(),
                value: value_temp,
                ty: ty.clone(),
            });
            field_types.insert(name.clone(), ty);
        }
        self.closure_env_types.insert(env_struct_name, field_types);

        let closure_temp = self.new_temp();
        self.push_instruction(Instruction::Alloc {
            result: closure_temp.clone(),
            ty: "Closure".to_string(),
            size: None,
        });
        self.push_instruction(Instruction::SetField {
            object: closure_temp.clone(),
            field: "fn_ptr".to_string(),
            // Passing the bare lifted-function name directly (not wrapped
            // in a temp) is intentional: koi-assembly's codegen already
            // falls back to `leaq function_symbol(%rip), reg` whenever an
            // operand name isn't a known local but IS a known top-level
            // function -- the same mechanism that already makes
            // non-capturing lambdas (passed as bare function values) work
            // today, so no codegen change is needed for this step.
            value: function_name.to_string(),
            ty: "i64".to_string(),
        });
        self.push_instruction(Instruction::SetField {
            object: closure_temp.clone(),
            field: "env_ptr".to_string(),
            value: env_temp,
            ty: "i64".to_string(),
        });

        Ok((closure_temp, format!("closure_{function_name}")))
    }

    fn generate_call(
        &mut self,
        function: &ASTNode,
        arguments: &[ASTNode],
    ) -> Result<(String, String), String> {
        if let ASTNode::Variable { name, .. } = function {
            if let Some(kind) = builtin_kind(name) {
                return self.generate_builtin_call(kind, name, arguments);
            }

            let mut arg_temps = vec![];
            for arg in arguments {
                let (temp, _) = self.generate_expr(arg)?;
                arg_temps.push(temp);
            }

            if self.functions.contains_key(name) {
                // A real top-level/lifted function: statically known name.
                let return_ty = match self.functions.get(name) {
                    Some(Type::Function { return_type, .. }) => return_type.mangled_name(),
                    Some(other) => other.mangled_name(),
                    None => "i64".to_string(),
                };
                let result = self.new_temp();
                self.push_instruction(Instruction::Call {
                    result: Some(result.clone()),
                    function: name.clone(),
                    arguments: arg_temps,
                    ty: Some(return_ty.clone()),
                });
                return Ok((result, return_ty));
            }

            // Not a known top-level function: this is a call through a
            // local variable/parameter holding a function value (e.g.
            // apply-func's `f`) -- a genuinely indirect call.
            let (function_value, ty) = self
                .lookup(name)
                .unwrap_or_else(|| (name.to_string(), "i64".to_string()));

            if ty.starts_with("closure_") {
                return Ok(self.generate_closure_call(function_value, &ty, arg_temps));
            }

            let return_ty = ty
                .strip_prefix("fn_")
                .and_then(|s| s.split("_to_").last())
                .unwrap_or("i64")
                .to_string();
            let result = self.new_temp();
            self.push_instruction(Instruction::CallIndirect {
                result: Some(result.clone()),
                function_value,
                arguments: arg_temps,
                ty: Some(return_ty.clone()),
            });
            return Ok((result, return_ty));
        }

        // Function position isn't a bare name -- this happens for an
        // immediately-invoked lambda expression (e.g. `((lambda [x] ...) 5)`),
        // which after lifting becomes `Call{function: MakeClosure{...}, ..}`.
        // Whatever it evaluates to is necessarily a value, not a static name,
        // so this is indirect too -- and if it's a closure value, it needs
        // the same fn_ptr/env_ptr unpacking as the named-variable case above.
        let (function_value, ty) = self.generate_expr(function)?;
        let mut arg_temps = vec![];
        for arg in arguments {
            let (temp, _) = self.generate_expr(arg)?;
            arg_temps.push(temp);
        }

        if ty.starts_with("closure_") {
            return Ok(self.generate_closure_call(function_value, &ty, arg_temps));
        }

        let result = self.new_temp();
        self.push_instruction(Instruction::CallIndirect {
            result: Some(result.clone()),
            function_value,
            arguments: arg_temps,
            ty: Some(ty.clone()),
        });
        Ok((result, ty))
    }

    /// Unpacks a `Closure` value's `fn_ptr`/`env_ptr` fields and emits the
    /// underlying indirect call with `env_ptr` prepended to the arguments,
    /// matching `lambda_lifter.rs`'s convention of making `env` the lifted
    /// function's own first parameter. `ty` must have a `"closure_"` prefix
    /// (the caller checks this); the suffix names the lifted function so its
    /// real return type can be looked up.
    fn generate_closure_call(
        &mut self,
        closure_value: String,
        ty: &str,
        arg_temps: Vec<String>,
    ) -> (String, String) {
        let closure_function_name = ty.strip_prefix("closure_").unwrap_or(ty).to_string();

        let fn_ptr_temp = self.new_temp();
        self.push_instruction(Instruction::GetField {
            result: fn_ptr_temp.clone(),
            object: closure_value.clone(),
            field: "fn_ptr".to_string(),
            ty: "i64".to_string(),
        });
        let env_ptr_temp = self.new_temp();
        self.push_instruction(Instruction::GetField {
            result: env_ptr_temp.clone(),
            object: closure_value,
            field: "env_ptr".to_string(),
            ty: "i64".to_string(),
        });

        let mut full_args = vec![env_ptr_temp];
        full_args.extend(arg_temps);

        let return_ty = match self.functions.get(&closure_function_name) {
            Some(Type::Function { return_type, .. }) => return_type.mangled_name(),
            _ => "i64".to_string(),
        };

        let result = self.new_temp();
        self.push_instruction(Instruction::CallIndirect {
            result: Some(result.clone()),
            function_value: fn_ptr_temp,
            arguments: full_args,
            ty: Some(return_ty.clone()),
        });
        (result, return_ty)
    }

    fn generate_builtin_call(
        &mut self,
        kind: BuiltinKind,
        name: &str,
        arguments: &[ASTNode],
    ) -> Result<(String, String), String> {
        let mut arg_pairs = vec![];
        for arg in arguments {
            arg_pairs.push(self.generate_expr(arg)?);
        }

        match kind {
            BuiltinKind::Arith => self.generate_arith(name, arg_pairs),
            BuiltinKind::Cmp => self.generate_fold(name, &arg_pairs, "bool", true),
            BuiltinKind::Logical => self.generate_fold(name, &arg_pairs, "bool", false),
            BuiltinKind::Not => self.generate_not(arg_pairs),
            BuiltinKind::Print | BuiltinKind::Malloc | BuiltinKind::Free => {
                let arg_temps: Vec<String> = arg_pairs.iter().map(|(t, _)| t.clone()).collect();
                let ty = if kind == BuiltinKind::Malloc {
                    "ptr_i64".to_string()
                } else {
                    "i64".to_string()
                };
                let result = self.new_temp();
                self.push_instruction(Instruction::Call {
                    result: Some(result.clone()),
                    function: name.to_string(),
                    arguments: arg_temps,
                    ty: Some(ty.clone()),
                });
                Ok((result, ty))
            }
            BuiltinKind::SetIndex => {
                let mut iter = arg_pairs.into_iter();
                let (array_temp, _) = iter
                    .next()
                    .ok_or_else(|| "aset! needs 3 arguments".to_string())?;
                let (index_temp, _) = iter
                    .next()
                    .ok_or_else(|| "aset! needs 3 arguments".to_string())?;
                let (value_temp, value_ty) = iter
                    .next()
                    .ok_or_else(|| "aset! needs 3 arguments".to_string())?;
                self.push_instruction(Instruction::SetIndex {
                    array: array_temp,
                    index: index_temp,
                    value: value_temp,
                    ty: value_ty,
                });
                Ok(self.unit_result())
            }
        }
    }

    fn generate_arith(
        &mut self,
        op: &str,
        arg_pairs: Vec<(String, String)>,
    ) -> Result<(String, String), String> {
        match arg_pairs.len() {
            0 => {
                let result = self.new_temp();
                self.push_instruction(Instruction::Const {
                    result: result.clone(),
                    value: serde_json::json!(0),
                    ty: "i64".into(),
                });
                Ok((result, "i64".to_string()))
            }
            1 => {
                let (temp, ty) = arg_pairs.into_iter().next().expect("checked len == 1");
                if op == "-" {
                    let zero = self.new_temp();
                    self.push_instruction(Instruction::Const {
                        result: zero.clone(),
                        value: serde_json::json!(0),
                        ty: ty.clone(),
                    });
                    let result = self.new_temp();
                    self.push_instruction(Instruction::BinOp {
                        result: result.clone(),
                        lhs: zero,
                        rhs: temp,
                        op_type: "-".to_string(),
                        ty: ty.clone(),
                    });
                    Ok((result, ty))
                } else {
                    Ok((temp, ty))
                }
            }
            _ => {
                let (mut acc_temp, mut acc_ty) = arg_pairs[0].clone();
                for (temp, ty) in &arg_pairs[1..] {
                    let result = self.new_temp();
                    self.push_instruction(Instruction::BinOp {
                        result: result.clone(),
                        lhs: acc_temp.clone(),
                        rhs: temp.clone(),
                        op_type: op.to_string(),
                        ty: acc_ty.clone(),
                    });
                    acc_temp = result;
                    acc_ty = ty.clone();
                }
                Ok((acc_temp, acc_ty))
            }
        }
    }

    /// Folds a chain of comparison/logical operands pairwise into `BinOp`s
    /// sharing `op`, defaulting to `default_value` when there are fewer
    /// than two operands.
    fn generate_fold(
        &mut self,
        op: &str,
        arg_pairs: &[(String, String)],
        result_ty: &str,
        default_value: bool,
    ) -> Result<(String, String), String> {
        if arg_pairs.len() < 2 {
            // Degenerate arity (0 or 1 operands): nothing to fold against,
            // so fall back to a default constant. Doesn't occur in this
            // project's test programs.
            let result = self.new_temp();
            self.push_instruction(Instruction::Const {
                result: result.clone(),
                value: serde_json::json!(default_value),
                ty: result_ty.to_string(),
            });
            return Ok((result, result_ty.to_string()));
        }

        let mut acc_temp = arg_pairs[0].0.clone();
        for (temp, _) in &arg_pairs[1..] {
            let result = self.new_temp();
            self.push_instruction(Instruction::BinOp {
                result: result.clone(),
                lhs: acc_temp.clone(),
                rhs: temp.clone(),
                op_type: op.to_string(),
                ty: result_ty.to_string(),
            });
            acc_temp = result;
        }
        Ok((acc_temp, result_ty.to_string()))
    }

    fn generate_not(
        &mut self,
        arg_pairs: Vec<(String, String)>,
    ) -> Result<(String, String), String> {
        let (temp, _) = arg_pairs
            .into_iter()
            .next()
            .ok_or_else(|| "`!` needs one operand".to_string())?;
        let false_temp = self.new_temp();
        self.push_instruction(Instruction::Const {
            result: false_temp.clone(),
            value: serde_json::json!(false),
            ty: "bool".into(),
        });
        let result = self.new_temp();
        self.push_instruction(Instruction::BinOp {
            result: result.clone(),
            lhs: temp,
            rhs: false_temp,
            op_type: "==".to_string(),
            ty: "bool".into(),
        });
        Ok((result, "bool".to_string()))
    }

    /// Resolves a field's real type for a `FieldAccess { object, field }`,
    /// given the object's already-generated type `object_ty`. Checks
    /// `closure_env_types` first -- `object_ty` is an env-struct's
    /// synthetic type name (`format!("env_{function_name}")`) exactly when
    /// this `FieldAccess` is a captured-variable access rewritten by
    /// `lambda_lifter.rs`'s `rewrite_free_var_access` (`env.captured_name`)
    /// -- before falling back to the normal `struct_fields`-based path used
    /// for real user `defstruct`s, which knows nothing about env structs
    /// (they're synthesized by the lifter, never declared via `defstruct`).
    fn field_type(&self, object_ty: &str, field: &str) -> String {
        if let Some(fields) = self.closure_env_types.get(object_ty)
            && let Some(ty) = fields.get(field)
        {
            return ty.clone();
        }
        for fields in self.struct_fields.values() {
            if let Some((_, ty)) = fields.iter().find(|(name, _)| name == field) {
                return ty.mangled_name();
            }
        }
        "i64".to_string()
    }

    fn declare(&mut self, name: &str, temp: String, ty: String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), (temp, ty));
        }
    }

    fn lookup(&self, name: &str) -> Option<(String, String)> {
        for scope in self.scopes.iter().rev() {
            if let Some(binding) = scope.get(name) {
                return Some(binding.clone());
            }
        }
        None
    }

    /// Overwrites `name`'s binding in whichever scope currently owns it
    /// (searched innermost-to-outermost, exactly like `lookup`), for
    /// `set!`. Unlike `declare`, this never creates a new binding in the
    /// innermost scope -- it mutates the existing entry wherever it lives.
    /// koi-ast's scope analysis already rejects `set!` on an undeclared
    /// name before this stage runs, so the `Err` case shouldn't occur in
    /// practice, but this is a real function that must not panic if it did.
    fn reassign(&mut self, name: &str, temp: String, ty: String) -> Result<(), String> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(entry) = scope.get_mut(name) {
                *entry = (temp, ty);
                return Ok(());
            }
        }
        Err(format!("cannot set! undefined variable '{name}'"))
    }

    fn new_temp(&mut self) -> String {
        let temp = format!("%v{}", self.temp_counter);
        self.temp_counter += 1;
        temp
    }

    fn new_label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.label_counter);
        self.label_counter += 1;
        label
    }

    fn push_instruction(&mut self, instruction: Instruction) {
        self.current_instructions.push(instruction);
    }

    fn finish_block(&mut self) {
        let label = self.current_label.clone();
        let instructions = std::mem::take(&mut self.current_instructions);
        self.blocks.push(BasicBlock {
            label,
            instructions,
        });
    }
}

fn elem_type_of(arr_ty: &str) -> String {
    arr_ty.strip_prefix("arr_").unwrap_or("i64").to_string()
}

fn literal_ir_type(literal_type: &str) -> String {
    match literal_type {
        "int64" => "i64".to_string(),
        "float64" => "f64".to_string(),
        "bool" => "bool".to_string(),
        "string" => "string".to_string(),
        other => other.to_string(),
    }
}

fn default_value_for_type(ty: &str) -> serde_json::Value {
    match ty {
        "i64" => serde_json::json!(0),
        "f64" => serde_json::json!(0.0),
        "bool" => serde_json::json!(false),
        "string" => serde_json::json!(""),
        _ => serde_json::Value::Null,
    }
}
