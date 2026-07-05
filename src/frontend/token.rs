use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Token {
    // Delimiters
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // Literals
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    StringLiteral(String),

    // Symbols and identifiers: +, -, *, /, foo, defn, if, let, loop, lambda, etc.
    Symbol(String),

    // Special
    Colon,
    Arrow,     // ->
    Ampersand, // & (address-of, when not part of &&)
    Asterisk,  // * (dereference, when not part of ** or read as a binary call head)

    // Control
    Eof,
}

#[derive(Debug, Clone)]
pub struct TokenWithPos {
    pub token: Token,
    pub line: usize,
    pub column: usize,
}
