//! Compiler-intrinsic operator classification, shared by `inference.rs`
//! (which needs precise type rules for them) and `ir_generator.rs` (which
//! needs to lower them to `binop`/`call` instructions). There is no `defn`
//! anywhere for `+`, `print`, etc., so both stages special-case these by
//! name rather than treating them as ordinary looked-up function values.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinKind {
    Arith,
    Cmp,
    Logical,
    Not,
    Print,
    Malloc,
    Free,
    SetIndex,
}

pub fn builtin_kind(name: &str) -> Option<BuiltinKind> {
    match name {
        "+" | "-" | "*" | "/" => Some(BuiltinKind::Arith),
        "<" | "<=" | ">" | ">=" | "==" | "!=" => Some(BuiltinKind::Cmp),
        "&&" | "||" => Some(BuiltinKind::Logical),
        "!" => Some(BuiltinKind::Not),
        "print" => Some(BuiltinKind::Print),
        "malloc" => Some(BuiltinKind::Malloc),
        "free" => Some(BuiltinKind::Free),
        "aset!" => Some(BuiltinKind::SetIndex),
        _ => None,
    }
}

pub const BUILTIN_NAMES: &[&str] = &[
    "+", "-", "*", "/", "<", "<=", ">", ">=", "==", "!=", "&&", "||", "!", "print", "malloc",
    "free", "aset!",
];
