use crate::middle_end::ir::IRProgram;

/// Generates ARM64 assembly from the IR.
/// This is a stub — full implementation pending.
pub fn generate_arm64(_program: &IRProgram) -> Result<String, String> {
    Err("ARM64 backend is not yet implemented".to_string())
}
