use crate::frontend::ast::ASTNode;
use crate::middle_end::builtins::BUILTIN_NAMES;
use std::collections::HashSet;

pub struct LambdaLifter {
    lifted_functions: Vec<ASTNode>,
    lambda_counter: usize,
    globals: HashSet<String>,
}

impl LambdaLifter {
    pub fn new(top_level_function_names: HashSet<String>) -> Self {
        let mut globals = top_level_function_names;
        globals.extend(BUILTIN_NAMES.iter().map(|s| s.to_string()));
        LambdaLifter {
            lifted_functions: vec![],
            lambda_counter: 0,
            globals,
        }
    }

    /// Scans `program` for top-level function names to seed `globals`
    /// automatically.
    pub fn for_program(program: &ASTNode) -> Self {
        let mut names = HashSet::new();
        if let ASTNode::Program { children } = program {
            for child in children {
                if let ASTNode::FunctionDef { name, .. } = child {
                    names.insert(name.clone());
                }
            }
        }
        Self::new(names)
    }

    /// Lifts every lambda in `program` to a top-level function, returning a
    /// new program with the lifted functions prepended.
    pub fn lift_program(&mut self, program: &ASTNode) -> ASTNode {
        let rewritten = self.lift_node(program);
        match rewritten {
            ASTNode::Program { children } => {
                let mut all = std::mem::take(&mut self.lifted_functions);
                all.extend(children);
                ASTNode::Program { children: all }
            }
            other => other,
        }
    }

    fn lift_node(&mut self, node: &ASTNode) -> ASTNode {
        match node {
            ASTNode::Program { children } => ASTNode::Program {
                children: children.iter().map(|c| self.lift_node(c)).collect(),
            },
            ASTNode::FunctionDef {
                name,
                parameters,
                body,
                line,
                column,
            } => ASTNode::FunctionDef {
                name: name.clone(),
                parameters: parameters.clone(),
                body: Box::new(self.lift_node(body)),
                line: *line,
                column: *column,
            },
            ASTNode::StructDef { .. } | ASTNode::Variable { .. } | ASTNode::Literal { .. } => {
                node.clone()
            }

            // `MakeClosure` is itself the OUTPUT of processing a `Lambda`
            // node below (see `lift_lambda`), not something that appears as
            // an input containing further sub-expressions of its own to
            // recurse into -- it's already a fully "lifted" leaf by
            // construction, so this is a pass-through purely for match
            // exhaustiveness over the new `ASTNode` variant.
            ASTNode::MakeClosure { .. } => node.clone(),

            ASTNode::Lambda {
                parameters,
                body,
                line,
                column,
            } => self.lift_lambda(parameters, body, *line, *column),

            ASTNode::Call {
                function,
                arguments,
                line,
                column,
            } => ASTNode::Call {
                function: Box::new(self.lift_node(function)),
                arguments: arguments.iter().map(|a| self.lift_node(a)).collect(),
                line: *line,
                column: *column,
            },
            ASTNode::LetBinding {
                bindings,
                body,
                line,
                column,
            } => ASTNode::LetBinding {
                bindings: bindings
                    .iter()
                    .map(|(n, v)| (n.clone(), Box::new(self.lift_node(v))))
                    .collect(),
                body: Box::new(self.lift_node(body)),
                line: *line,
                column: *column,
            },
            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                line,
                column,
            } => ASTNode::IfExpr {
                condition: Box::new(self.lift_node(condition)),
                then_branch: Box::new(self.lift_node(then_branch)),
                else_branch: else_branch.as_ref().map(|e| Box::new(self.lift_node(e))),
                line: *line,
                column: *column,
            },
            ASTNode::LoopExpr {
                variable,
                init,
                condition,
                step,
                body,
                line,
                column,
            } => ASTNode::LoopExpr {
                variable: variable.clone(),
                init: Box::new(self.lift_node(init)),
                condition: Box::new(self.lift_node(condition)),
                step: Box::new(self.lift_node(step)),
                body: Box::new(self.lift_node(body)),
                line: *line,
                column: *column,
            },
            ASTNode::FieldAccess {
                object,
                field,
                line,
                column,
            } => ASTNode::FieldAccess {
                object: Box::new(self.lift_node(object)),
                field: field.clone(),
                line: *line,
                column: *column,
            },
            ASTNode::SetField {
                object,
                field,
                value,
                line,
                column,
            } => ASTNode::SetField {
                object: Box::new(self.lift_node(object)),
                field: field.clone(),
                value: Box::new(self.lift_node(value)),
                line: *line,
                column: *column,
            },
            ASTNode::Index {
                array,
                index,
                line,
                column,
            } => ASTNode::Index {
                array: Box::new(self.lift_node(array)),
                index: Box::new(self.lift_node(index)),
                line: *line,
                column: *column,
            },
            ASTNode::AddrOf {
                operand,
                line,
                column,
            } => ASTNode::AddrOf {
                operand: Box::new(self.lift_node(operand)),
                line: *line,
                column: *column,
            },
            ASTNode::Deref {
                operand,
                line,
                column,
            } => ASTNode::Deref {
                operand: Box::new(self.lift_node(operand)),
                line: *line,
                column: *column,
            },
            ASTNode::New {
                type_str,
                size_or_init,
                line,
                column,
            } => ASTNode::New {
                type_str: type_str.clone(),
                size_or_init: size_or_init.as_ref().map(|e| Box::new(self.lift_node(e))),
                line: *line,
                column: *column,
            },
            ASTNode::ArrayLiteral {
                elements,
                line,
                column,
            } => ASTNode::ArrayLiteral {
                elements: elements.iter().map(|e| self.lift_node(e)).collect(),
                line: *line,
                column: *column,
            },
            ASTNode::SetVar {
                name,
                value,
                line,
                column,
            } => ASTNode::SetVar {
                name: name.clone(),
                value: Box::new(self.lift_node(value)),
                line: *line,
                column: *column,
            },
            ASTNode::WhileExpr {
                condition,
                body,
                line,
                column,
            } => ASTNode::WhileExpr {
                condition: Box::new(self.lift_node(condition)),
                body: Box::new(self.lift_node(body)),
                line: *line,
                column: *column,
            },
            ASTNode::DoExpr {
                exprs,
                line,
                column,
            } => ASTNode::DoExpr {
                exprs: exprs.iter().map(|e| self.lift_node(e)).collect(),
                line: *line,
                column: *column,
            },
        }
    }

    fn lift_lambda(
        &mut self,
        parameters: &[(String, Option<String>)],
        body: &ASTNode,
        line: usize,
        column: usize,
    ) -> ASTNode {
        // Lift any nested lambdas first, so their own captures are already
        // resolved to plain Variable references (or closure-construction
        // calls) by the time we look for *this* lambda's free variables.
        let lifted_body = self.lift_node(body);

        let bound: HashSet<String> = parameters.iter().map(|(n, _)| n.clone()).collect();
        let mut free_vars = HashSet::new();
        collect_free_variables(&lifted_body, &bound, &self.globals, &mut free_vars);

        let id = self.lambda_counter;
        self.lambda_counter += 1;
        let func_name = format!("_lambda_{id}");

        // Register this lambda's lifted name as a global *before* any
        // enclosing lambda gets to analyze its own free variables --
        // otherwise a reference to this lambda's lifted name, appearing in
        // an *outer* lambda's body (e.g. as the `function_name` of a nested
        // `MakeClosure`), would be mistaken for a captured variable instead
        // of a reference to a global function. (There used to be a second
        // insertion here for a `__make_closure_{func_name}` placeholder
        // global -- that's gone now along with the placeholder `Call` node
        // it named, since capturing lambdas lower to a dedicated
        // `MakeClosure` node below instead of a call to a made-up function
        // name that nothing ever defined.)
        self.globals.insert(func_name.clone());

        if free_vars.is_empty() {
            self.lifted_functions.push(ASTNode::FunctionDef {
                name: func_name.clone(),
                parameters: parameters.to_vec(),
                body: Box::new(lifted_body),
                line,
                column,
            });
            // No captures -- the lifted function's name stands in directly
            // for the lambda value.
            return ASTNode::Variable {
                name: func_name,
                line,
                column,
            };
        }

        // Captures path. Field types below are still hardcoded to i64 in
        // this StructDef -- that's legacy/vestigial at this point and isn't
        // consulted by `ir_generator.rs` for anything real: real closure
        // construction (env struct alloc + field stores with each
        // captured variable's *actual* type + the shared `Closure` wrapper)
        // happens in `ir_generator.rs`'s `MakeClosure` handling, which runs
        // after monomorphization and lambda-lifting, so it has real,
        // concrete types available via its own `self.lookup` -- unlike
        // here, where only names are known. This StructDef is kept only so
        // that existing tests asserting an env struct's *field names* (not
        // types) keep passing; nothing downstream depends on its declared
        // field types.
        let env_struct_name = format!("_Lambda_{id}_Env");
        let mut captured: Vec<String> = free_vars.iter().cloned().collect();
        captured.sort();

        self.lifted_functions.push(ASTNode::StructDef {
            name: env_struct_name.clone(),
            fields: captured
                .iter()
                .map(|v| (v.clone(), "i64".to_string()))
                .collect(),
            line,
            column,
        });

        let mut lifted_params = vec![("env".to_string(), Some(env_struct_name))];
        lifted_params.extend(parameters.iter().cloned());

        let rewritten_body = rewrite_free_var_access(&lifted_body, &free_vars);

        self.lifted_functions.push(ASTNode::FunctionDef {
            name: func_name.clone(),
            parameters: lifted_params,
            body: Box::new(rewritten_body),
            line,
            column,
        });

        // Actual closure construction (env struct alloc + field stores +
        // the shared two-field `Closure` wrapper) happens in
        // `ir_generator.rs`, which runs strictly after monomorphization --
        // this stage only has captured *names*, not types, so it just marks
        // "construct a closure here, capturing these names" via this
        // dedicated node rather than emitting a placeholder call to a
        // function name nothing ever defines.
        ASTNode::MakeClosure {
            function_name: func_name.clone(),
            captured,
            line,
            column,
        }
    }
}

fn collect_free_variables(
    node: &ASTNode,
    bound: &HashSet<String>,
    globals: &HashSet<String>,
    free: &mut HashSet<String>,
) {
    match node {
        ASTNode::Variable { name, .. } => {
            if !bound.contains(name) && !globals.contains(name) {
                free.insert(name.clone());
            }
        }
        ASTNode::Literal { .. } => {}
        ASTNode::Call {
            function,
            arguments,
            ..
        } => {
            collect_free_variables(function, bound, globals, free);
            for arg in arguments {
                collect_free_variables(arg, bound, globals, free);
            }
        }
        ASTNode::LetBinding { bindings, body, .. } => {
            let mut inner_bound = bound.clone();
            for (name, value) in bindings {
                collect_free_variables(value, &inner_bound, globals, free);
                inner_bound.insert(name.clone());
            }
            collect_free_variables(body, &inner_bound, globals, free);
        }
        ASTNode::IfExpr {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            collect_free_variables(condition, bound, globals, free);
            collect_free_variables(then_branch, bound, globals, free);
            if let Some(e) = else_branch {
                collect_free_variables(e, bound, globals, free);
            }
        }
        ASTNode::LoopExpr {
            variable,
            init,
            condition,
            step,
            body,
            ..
        } => {
            collect_free_variables(init, bound, globals, free);
            let mut inner_bound = bound.clone();
            inner_bound.insert(variable.clone());
            collect_free_variables(condition, &inner_bound, globals, free);
            collect_free_variables(step, &inner_bound, globals, free);
            collect_free_variables(body, &inner_bound, globals, free);
        }
        ASTNode::Lambda {
            parameters, body, ..
        } => {
            let mut inner_bound = bound.clone();
            for (name, _) in parameters {
                inner_bound.insert(name.clone());
            }
            collect_free_variables(body, &inner_bound, globals, free);
        }
        ASTNode::FieldAccess { object, .. } => collect_free_variables(object, bound, globals, free),
        ASTNode::SetField { object, value, .. } => {
            collect_free_variables(object, bound, globals, free);
            collect_free_variables(value, bound, globals, free);
        }
        ASTNode::Index { array, index, .. } => {
            collect_free_variables(array, bound, globals, free);
            collect_free_variables(index, bound, globals, free);
        }
        ASTNode::AddrOf { operand, .. } | ASTNode::Deref { operand, .. } => {
            collect_free_variables(operand, bound, globals, free);
        }
        ASTNode::New { size_or_init, .. } => {
            if let Some(e) = size_or_init {
                collect_free_variables(e, bound, globals, free);
            }
        }
        ASTNode::ArrayLiteral { elements, .. } => {
            for e in elements {
                collect_free_variables(e, bound, globals, free);
            }
        }
        ASTNode::Program { .. } | ASTNode::FunctionDef { .. } | ASTNode::StructDef { .. } => {}
        ASTNode::MakeClosure { captured, .. } => {
            // Each captured name is a *use*, exactly like a bare `Variable`
            // reference -- relevant for the rare case of a lambda nested
            // inside another capturing lambda, where an inner
            // `MakeClosure`'s captured name may itself need to be captured
            // by the *outer* lambda currently being analyzed.
            for name in captured {
                if !bound.contains(name) && !globals.contains(name) {
                    free.insert(name.clone());
                }
            }
        }
        ASTNode::SetVar { name, value, .. } => {
            // The assignment target is itself a use, exactly like a bare
            // `Variable` reference -- it must be captured if it isn't a
            // local binding or a global.
            if !bound.contains(name) && !globals.contains(name) {
                free.insert(name.clone());
            }
            collect_free_variables(value, bound, globals, free);
        }
        ASTNode::WhileExpr {
            condition, body, ..
        } => {
            // Unlike `loop`, `while` introduces no binding of its own.
            collect_free_variables(condition, bound, globals, free);
            collect_free_variables(body, bound, globals, free);
        }
        ASTNode::DoExpr { exprs, .. } => {
            for e in exprs {
                collect_free_variables(e, bound, globals, free);
            }
        }
    }
}

fn rewrite_free_var_access(node: &ASTNode, free_vars: &HashSet<String>) -> ASTNode {
    match node {
        ASTNode::Variable { name, line, column } => {
            if free_vars.contains(name) {
                ASTNode::FieldAccess {
                    object: Box::new(ASTNode::Variable {
                        name: "env".to_string(),
                        line: *line,
                        column: *column,
                    }),
                    field: name.clone(),
                    line: *line,
                    column: *column,
                }
            } else {
                node.clone()
            }
        }
        ASTNode::Literal { .. }
        | ASTNode::StructDef { .. }
        | ASTNode::Program { .. }
        | ASTNode::FunctionDef { .. } => node.clone(),
        ASTNode::MakeClosure {
            function_name,
            captured,
            line,
            column,
        } => {
            // NOTE (nested-closure limitation, deliberately not solved
            // here): `captured` is a list of plain variable *names*, not
            // `ASTNode`s -- there's no sub-node here to swap in a
            // `FieldAccess { object: env, .. }` the way a bare `Variable`
            // reference gets rewritten above. If one of these names is
            // *itself* a free variable of an OUTER capturing lambda (i.e. a
            // lambda capturing a variable that itself contains another
            // capturing lambda), this inner `MakeClosure`'s `captured` list
            // would need to source that name from the outer lambda's own
            // `env` rather than a plain local for the generated code to be
            // correct -- that genuinely-nested-closures case is out of
            // scope for this MVP and is left unhandled. The common,
            // single-level case needs no rewrite here: `captured` already
            // only contains names that are genuinely local to the function
            // this `MakeClosure` sits inside.
            ASTNode::MakeClosure {
                function_name: function_name.clone(),
                captured: captured.clone(),
                line: *line,
                column: *column,
            }
        }
        ASTNode::Call {
            function,
            arguments,
            line,
            column,
        } => ASTNode::Call {
            function: Box::new(rewrite_free_var_access(function, free_vars)),
            arguments: arguments
                .iter()
                .map(|a| rewrite_free_var_access(a, free_vars))
                .collect(),
            line: *line,
            column: *column,
        },
        ASTNode::LetBinding {
            bindings,
            body,
            line,
            column,
        } => {
            // A let-bound name shadows a captured free variable of the same
            // name for the rest of the let.
            let mut still_free = free_vars.clone();
            let mut new_bindings = vec![];
            for (name, value) in bindings {
                new_bindings.push((
                    name.clone(),
                    Box::new(rewrite_free_var_access(value, &still_free)),
                ));
                still_free.remove(name);
            }
            ASTNode::LetBinding {
                bindings: new_bindings,
                body: Box::new(rewrite_free_var_access(body, &still_free)),
                line: *line,
                column: *column,
            }
        }
        ASTNode::IfExpr {
            condition,
            then_branch,
            else_branch,
            line,
            column,
        } => ASTNode::IfExpr {
            condition: Box::new(rewrite_free_var_access(condition, free_vars)),
            then_branch: Box::new(rewrite_free_var_access(then_branch, free_vars)),
            else_branch: else_branch
                .as_ref()
                .map(|e| Box::new(rewrite_free_var_access(e, free_vars))),
            line: *line,
            column: *column,
        },
        ASTNode::LoopExpr {
            variable,
            init,
            condition,
            step,
            body,
            line,
            column,
        } => {
            let mut inner_free = free_vars.clone();
            inner_free.remove(variable);
            ASTNode::LoopExpr {
                variable: variable.clone(),
                init: Box::new(rewrite_free_var_access(init, free_vars)),
                condition: Box::new(rewrite_free_var_access(condition, &inner_free)),
                step: Box::new(rewrite_free_var_access(step, &inner_free)),
                body: Box::new(rewrite_free_var_access(body, &inner_free)),
                line: *line,
                column: *column,
            }
        }
        ASTNode::Lambda {
            parameters,
            body,
            line,
            column,
        } => {
            let mut inner_free = free_vars.clone();
            for (name, _) in parameters {
                inner_free.remove(name);
            }
            ASTNode::Lambda {
                parameters: parameters.clone(),
                body: Box::new(rewrite_free_var_access(body, &inner_free)),
                line: *line,
                column: *column,
            }
        }
        ASTNode::FieldAccess {
            object,
            field,
            line,
            column,
        } => ASTNode::FieldAccess {
            object: Box::new(rewrite_free_var_access(object, free_vars)),
            field: field.clone(),
            line: *line,
            column: *column,
        },
        ASTNode::SetField {
            object,
            field,
            value,
            line,
            column,
        } => ASTNode::SetField {
            object: Box::new(rewrite_free_var_access(object, free_vars)),
            field: field.clone(),
            value: Box::new(rewrite_free_var_access(value, free_vars)),
            line: *line,
            column: *column,
        },
        ASTNode::Index {
            array,
            index,
            line,
            column,
        } => ASTNode::Index {
            array: Box::new(rewrite_free_var_access(array, free_vars)),
            index: Box::new(rewrite_free_var_access(index, free_vars)),
            line: *line,
            column: *column,
        },
        ASTNode::AddrOf {
            operand,
            line,
            column,
        } => ASTNode::AddrOf {
            operand: Box::new(rewrite_free_var_access(operand, free_vars)),
            line: *line,
            column: *column,
        },
        ASTNode::Deref {
            operand,
            line,
            column,
        } => ASTNode::Deref {
            operand: Box::new(rewrite_free_var_access(operand, free_vars)),
            line: *line,
            column: *column,
        },
        ASTNode::New {
            type_str,
            size_or_init,
            line,
            column,
        } => ASTNode::New {
            type_str: type_str.clone(),
            size_or_init: size_or_init
                .as_ref()
                .map(|e| Box::new(rewrite_free_var_access(e, free_vars))),
            line: *line,
            column: *column,
        },
        ASTNode::ArrayLiteral {
            elements,
            line,
            column,
        } => ASTNode::ArrayLiteral {
            elements: elements
                .iter()
                .map(|e| rewrite_free_var_access(e, free_vars))
                .collect(),
            line: *line,
            column: *column,
        },
        ASTNode::SetVar {
            name,
            value,
            line,
            column,
        } => {
            // Rewriting the *target* of a `set!` when it refers to a
            // captured/free variable (i.e. mutating a variable through a
            // closure's environment) is deliberately not handled here --
            // per the project's documented plan, closures capturing a
            // `set!`-mutated variable are unsupported/undefined behavior for
            // this MVP. Only the value expression is rewritten; `name`
            // passes through unchanged.
            ASTNode::SetVar {
                name: name.clone(),
                value: Box::new(rewrite_free_var_access(value, free_vars)),
                line: *line,
                column: *column,
            }
        }
        ASTNode::WhileExpr {
            condition,
            body,
            line,
            column,
        } => ASTNode::WhileExpr {
            condition: Box::new(rewrite_free_var_access(condition, free_vars)),
            body: Box::new(rewrite_free_var_access(body, free_vars)),
            line: *line,
            column: *column,
        },
        ASTNode::DoExpr {
            exprs,
            line,
            column,
        } => ASTNode::DoExpr {
            exprs: exprs
                .iter()
                .map(|e| rewrite_free_var_access(e, free_vars))
                .collect(),
            line: *line,
            column: *column,
        },
    }
}
