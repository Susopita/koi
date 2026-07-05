pub struct AArch64ABI;

impl AArch64ABI {
    pub const ARG_REGISTERS: [&'static str; 8] =
        ["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7"];
    pub const RETURN_REGISTER: &'static str = "x0";
    pub const SCRATCH0: &'static str = "x9";
    pub const SCRATCH1: &'static str = "x10";

    pub const FLOAT_ARG_REGISTERS: [&'static str; 8] =
        ["v0", "v1", "v2", "v3", "v4", "v5", "v6", "v7"];
    pub const FLOAT_RETURN_REGISTER: &'static str = "v0";

    pub fn arg_register(index: usize) -> Option<&'static str> { Self::ARG_REGISTERS.get(index).copied() }
    pub fn float_arg_register(index: usize) -> Option<&'static str> { Self::FLOAT_ARG_REGISTERS.get(index).copied() }

    pub fn stack_arg_offset(index: usize) -> Option<i64> {
        if index >= Self::ARG_REGISTERS.len() {
            Some(16 + 8 * (index - Self::ARG_REGISTERS.len()) as i64)
        } else { None }
    }

    pub fn align_to_16(size: i64) -> i64 {
        if size <= 0 { 0 } else { ((size + 15) / 16) * 16 }
    }
}
