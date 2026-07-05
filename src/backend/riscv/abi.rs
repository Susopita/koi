pub struct RiscVABI;

impl RiscVABI {
    pub const ARG_REGISTERS: [&'static str; 8] =
        ["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"];
    pub const RETURN_REGISTER: &'static str = "a0";
    pub const SCRATCH0: &'static str = "t0";
    pub const SCRATCH1: &'static str = "t1";

    pub fn arg_register(index: usize) -> Option<&'static str> { Self::ARG_REGISTERS.get(index).copied() }
    pub fn align_to_16(size: i64) -> i64 {
        if size <= 0 { 0 } else { ((size + 15) / 16) * 16 }
    }
}
