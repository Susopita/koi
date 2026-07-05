use crate::frontend::ast::ASTNode;
use crate::middle_end::inference::ConstraintGenerator;
use crate::middle_end::ir::IRProgram;
use crate::middle_end::ir_generator::IRGenerator;
use crate::middle_end::lambda_lifter::LambdaLifter;
use crate::middle_end::monomorphizer::Monomorphizer;
use crate::middle_end::types::Type;
use crate::middle_end::unification::Unifier;
use std::collections::HashMap;

/// Runs the full pipeline -- inference, unification, monomorphization,
/// lambda lifting, IR generation -- end to end. Shared by `main.rs` and the
/// integration tests so this orchestration exists in exactly one place;
/// errors are pre-tagged with their originating `[phase]`.
pub fn compile(program: &ASTNode) -> Result<IRProgram, String> {
    let mut generator = ConstraintGenerator::new();
    generator
        .generate_program(program)
        .map_err(|e| format!("[inference] {e}"))?;

    let subst =
        Unifier::unify(generator.constraints()).map_err(|e| format!("[unification] {e}"))?;

    let resolved_functions: HashMap<String, Type> = generator
        .functions()
        .iter()
        .map(|(name, ty)| (name.clone(), Unifier::resolve(&subst, ty)))
        .collect();

    let resolved_struct_fields: HashMap<String, Vec<(String, Type)>> = generator
        .struct_fields()
        .iter()
        .map(|(name, fields)| {
            let resolved = fields
                .iter()
                .map(|(fname, fty)| (fname.clone(), Unifier::resolve(&subst, fty)))
                .collect();
            (name.clone(), resolved)
        })
        .collect();

    let mut monomorphizer = Monomorphizer::new();
    monomorphizer.collect_from_functions(&resolved_functions);
    let program = monomorphizer.specialize_program(program);

    let mut lambda_lifter = LambdaLifter::for_program(&program);
    let program = lambda_lifter.lift_program(&program);

    // Lifted lambdas (e.g. `_lambda_0`) never went through inference -- they
    // didn't exist as named top-level functions until this step. Give them a
    // default signature (this MVP's Int64-everywhere fallback) so IR
    // generation can resolve calls/references to them.
    let mut resolved_functions = resolved_functions;
    add_default_signatures_for_new_functions(&program, &mut resolved_functions);

    IRGenerator::new(&resolved_functions, &resolved_struct_fields)
        .generate_program(&program)
        .map_err(|e| format!("[ir_generator] {e}"))
}

fn add_default_signatures_for_new_functions(
    program: &ASTNode,
    functions: &mut HashMap<String, Type>,
) {
    if let ASTNode::Program { children } = program {
        for child in children {
            if let ASTNode::FunctionDef {
                name, parameters, ..
            } = child
            {
                functions
                    .entry(name.clone())
                    .or_insert_with(|| Type::Function {
                        params: vec![Type::Int64; parameters.len()],
                        return_type: Box::new(Type::Int64),
                    });
            }
        }
    }
}
