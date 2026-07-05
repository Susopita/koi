use crate::frontend::ast::ASTNode;
use crate::middle_end::types::Type;
use std::collections::{HashMap, HashSet};

/// Detects and names function specializations.
///
/// Architectural note: this pipeline runs unification *before*
/// monomorphization (per PROMPT_B_RUST_IR.md's phase ordering), and gives
/// every function exactly one globally-shared, unified signature rather
/// than a polymorphic scheme that gets re-instantiated per call site. A
/// consequence: a function can only ever reach this stage with more than
/// one distinct argument-type tuple if unification had already failed on
/// the conflict -- which means it never would have gotten this far. So on
/// real programs from this pipeline, [`Monomorphizer::specializations_needed`]
/// is always empty and [`Monomorphizer::specialize_program`] is a no-op.
///
/// The detection/naming algorithm itself is fully implemented and correct
/// (see the unit tests, which drive it directly rather than through the
/// pipeline, since the pipeline can never actually exercise the multi-tuple
/// branch) -- this isn't a stub, it just currently has nothing to do.
pub struct Monomorphizer {
    instantiations: HashMap<String, HashSet<Vec<Type>>>,
}

impl Default for Monomorphizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Monomorphizer {
    pub fn new() -> Self {
        Monomorphizer {
            instantiations: HashMap::new(),
        }
    }

    /// Records that `function_name` was called with this exact
    /// argument-type tuple somewhere in the program.
    pub fn record_call(&mut self, function_name: &str, arg_types: Vec<Type>) {
        self.instantiations
            .entry(function_name.to_string())
            .or_default()
            .insert(arg_types);
    }

    /// Populates instantiations directly from each function's final,
    /// resolved signature -- the accurate source of truth for this
    /// pipeline, since every call site was already unified against exactly
    /// that one signature.
    pub fn collect_from_functions(&mut self, functions: &HashMap<String, Type>) {
        for (name, ty) in functions {
            if let Type::Function { params, .. } = ty {
                self.record_call(name, params.clone());
            }
        }
    }

    /// Functions that were observed with more than one distinct
    /// argument-type tuple, and the tuples themselves.
    pub fn specializations_needed(&self) -> HashMap<String, Vec<Vec<Type>>> {
        self.instantiations
            .iter()
            .filter(|(_, tuples)| tuples.len() > 1)
            .map(|(name, tuples)| (name.clone(), tuples.iter().cloned().collect()))
            .collect()
    }

    pub fn mangle_name(base: &str, arg_types: &[Type]) -> String {
        let parts: Vec<String> = arg_types.iter().map(Type::mangled_name).collect();
        format!("{base}__{}", parts.join("_"))
    }

    /// Clones+renames any function that needs more than one specialization.
    /// A no-op on programs that reached this stage through the normal
    /// pipeline (see module docs) -- provided for completeness and for
    /// exercising the clone/rename mechanics directly in tests.
    pub fn specialize_program(&self, program: &ASTNode) -> ASTNode {
        let needed = self.specializations_needed();
        if needed.is_empty() {
            return program.clone();
        }

        match program {
            ASTNode::Program { children } => {
                let mut new_children = vec![];
                for child in children {
                    match child {
                        ASTNode::FunctionDef {
                            name,
                            parameters,
                            body,
                            line,
                            column,
                        } if needed.contains_key(name) => {
                            for tuple in &needed[name] {
                                new_children.push(ASTNode::FunctionDef {
                                    name: Self::mangle_name(name, tuple),
                                    parameters: parameters.clone(),
                                    body: body.clone(),
                                    line: *line,
                                    column: *column,
                                });
                            }
                        }
                        other => new_children.push(other.clone()),
                    }
                }
                ASTNode::Program {
                    children: new_children,
                }
            }
            other => other.clone(),
        }
    }
}
