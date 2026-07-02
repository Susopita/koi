pub struct AMD64ABI;

impl AMD64ABI {
    pub const ARG_REGISTERS: [&'static str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
    pub const RETURN_REGISTER: &'static str = "%rax";
    pub const SCRATCH0: &'static str = "%r10";
    pub const SCRATCH1: &'static str = "%r11";
    pub const SCRATCH2: &'static str = "%rax";

    pub fn arg_register(index: usize) -> Option<&'static str> {
        Self::ARG_REGISTERS.get(index).copied()
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

