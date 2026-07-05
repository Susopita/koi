pub struct AMD64ABI;

impl AMD64ABI {
    pub const ARG_REGISTERS: [&'static str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
    pub const RETURN_REGISTER: &'static str = "%rax";
    pub const SCRATCH0: &'static str = "%r10";
    pub const SCRATCH1: &'static str = "%r11";
    pub const SCRATCH2: &'static str = "%rax";

    // Float/double (f64) counterparts of the integer ABI constants above.
    // System V AMD64: floating-point arguments are passed in %xmm0-%xmm7,
    // counted in a *separate* sequence from the integer argument registers
    // (e.g. `(a: i64, b: f64, c: i64)` passes a/%rdi, b/%xmm0, c/%rsi -- NOT
    // %rdx), and float return values come back in %xmm0, not %rax.
    pub const FLOAT_ARG_REGISTERS: [&'static str; 8] =
        ["%xmm0", "%xmm1", "%xmm2", "%xmm3", "%xmm4", "%xmm5", "%xmm6", "%xmm7"];
    pub const FLOAT_RETURN_REGISTER: &'static str = "%xmm0";
    pub const FLOAT_SCRATCH0: &'static str = "%xmm8";
    pub const FLOAT_SCRATCH1: &'static str = "%xmm9";

    pub fn arg_register(index: usize) -> Option<&'static str> {
        Self::ARG_REGISTERS.get(index).copied()
    }

    pub fn float_arg_register(index: usize) -> Option<&'static str> {
        Self::FLOAT_ARG_REGISTERS.get(index).copied()
    }

    pub fn stack_arg_offset(index: usize) -> Option<i64> {
        if index >= Self::ARG_REGISTERS.len() {
            Some(16 + 8 * (index - Self::ARG_REGISTERS.len()) as i64)
        } else {
            None
        }
    }

    pub fn align_to_16(size: i64) -> i64 {
        if size <= 0 {
            0
        } else {
            ((size + 15) / 16) * 16
        }
    }
}

