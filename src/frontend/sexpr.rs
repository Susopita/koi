/// An S-Expression representing the surface syntax of Carp.
///
/// This is a pure syntactic tree — no type information, no scope analysis,
/// no semantic validation.  It is the output of the `Reader` stage and the
/// input to the `Parser` stage.
#[derive(Debug, Clone, PartialEq)]
pub enum SExpr {
    /// A named symbol (operator, identifier, keyword, etc.).
    Symbol(String),
    /// A 64-bit signed integer literal.
    Integer(i64),
    /// A 64-bit floating-point literal.
    Float(f64),
    /// A string literal.
    String(String),
    /// A boolean literal.
    Bool(bool),
    /// A parenthesised / bracketed list — the fundamental compound form.
    List(Vec<SExpr>),
}

impl std::fmt::Display for SExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SExpr::Symbol(s) => write!(f, "{s}"),
            SExpr::Integer(n) => write!(f, "{n}"),
            SExpr::Float(n) => write!(f, "{n}"),
            SExpr::String(s) => write!(f, "\"{s}\""),
            SExpr::Bool(true) => write!(f, "#t"),
            SExpr::Bool(false) => write!(f, "#f"),
            SExpr::List(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reader — token stream → S-Expression tree
// ---------------------------------------------------------------------------

use crate::frontend::scanner::Scanner;
use crate::frontend::token::{Token, TokenWithPos};

/// Adapter that wraps [`Scanner`] as an iterator of [`TokenWithPos`],
/// stripping the trailing `Eof` sentinel so the reader sees a clean stream.
struct TokenStream<'a> {
    scanner: &'a mut Scanner,
    eof_sent: bool,
}

impl<'a> TokenStream<'a> {
    fn new(scanner: &'a mut Scanner) -> Self {
        TokenStream { scanner, eof_sent: false }
    }
}

impl Iterator for TokenStream<'_> {
    type Item = TokenWithPos;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof_sent {
            return None;
        }
        let twp = self.scanner.next_token();
        if matches!(twp.token, Token::Eof) {
            self.eof_sent = true;
            return None;
        }
        Some(twp)
    }
}

/// Reads a stream of tokens into a sequence of top-level S-Expressions.
///
/// The reader is purely structural: it groups tokens by matching
/// `LParen`/`RParen`, `LBracket`/`RBracket`, `LBrace`/`RBrace`.  It does
/// **not** perform any semantic analysis (no type checking, no scope
/// resolution, no macro expansion).
pub struct Reader<I> {
    tokens: I,
    buffer: Option<TokenWithPos>,
}

impl<I> Reader<I>
where
    I: Iterator<Item = TokenWithPos>,
{
    pub fn new(tokens: I) -> Self {
        Reader { tokens, buffer: None }
    }

    /// Read all top-level S-Expressions from the token stream.
    pub fn read_all(&mut self) -> Result<Vec<SExpr>, String> {
        let mut forms = Vec::new();
        loop {
            match self.peek_token()? {
                None => break,
                Some(Token::RParen)
                | Some(Token::RBracket)
                | Some(Token::RBrace) => {
                    let twp = self.next_token()?;
                    return Err(format!(
                        "unexpected closing delimiter at line {}, column {}",
                        twp.line, twp.column,
                    ));
                }
                _ => forms.push(self.read_one()?),
            }
        }
        Ok(forms)
    }

    /// Read a single S-Expression from the token stream.
    pub fn read_one(&mut self) -> Result<SExpr, String> {
        let tok = self.next_token()?;

        match tok.token {
            Token::LParen | Token::LBracket | Token::LBrace => {
                let mut items = Vec::new();
                loop {
                    match self.peek_token()? {
                        None => {
                            return Err(format!(
                                "unexpected end of input: opened at line {}, column {}",
                                tok.line, tok.column,
                            ));
                        }
                        Some(Token::RParen)
                        | Some(Token::RBracket)
                        | Some(Token::RBrace) => {
                            self.next_token()?;
                            break;
                        }
                        _ => items.push(self.read_one()?),
                    }
                }
                Ok(SExpr::List(items))
            }

            Token::RParen => Err(format!(
                "unexpected closing parenthesis at line {}, column {}",
                tok.line, tok.column,
            )),
            Token::RBracket => Err(format!(
                "unexpected closing bracket at line {}, column {}",
                tok.line, tok.column,
            )),
            Token::RBrace => Err(format!(
                "unexpected closing brace at line {}, column {}",
                tok.line, tok.column,
            )),

            Token::Symbol(s) => Ok(SExpr::Symbol(s)),
            Token::IntLiteral(n) => Ok(SExpr::Integer(n)),
            Token::FloatLiteral(f) => Ok(SExpr::Float(f)),
            Token::BoolLiteral(b) => Ok(SExpr::Bool(b)),
            Token::StringLiteral(s) => Ok(SExpr::String(s)),

            // Syntactic sugar tokens mapped to their symbol representation.
            // The semantic meaning is resolved when this SExpr is lowered to
            // an ASTNode by the Parser / type-checker.
            Token::Colon => Ok(SExpr::Symbol(":".to_string())),
            Token::Arrow => Ok(SExpr::Symbol("->".to_string())),
            Token::Ampersand => Ok(SExpr::Symbol("&".to_string())),
            Token::Asterisk => Ok(SExpr::Symbol("*".to_string())),

            Token::Eof => Err("unexpected end of file".to_string()),
        }
    }

    // -- helpers ----------------------------------------------------------

    fn peek_token(&mut self) -> Result<Option<&Token>, String> {
        self.fill_buffer()?;
        Ok(self.buffer.as_ref().map(|twp| &twp.token))
    }

    fn next_token(&mut self) -> Result<TokenWithPos, String> {
        self.fill_buffer()?;
        self.buffer.take().ok_or_else(|| "unexpected end of file".to_string())
    }

    fn fill_buffer(&mut self) -> Result<(), String> {
        if self.buffer.is_some() {
            return Ok(());
        }
        self.buffer = self.tokens.next();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience: read a source string directly
// ---------------------------------------------------------------------------

/// Parse a Carp source string into a vector of top-level S-Expressions.
/// This is the main entry point for the reader stage.
pub fn read_source(source: &str) -> Result<Vec<SExpr>, String> {
    let mut scanner = Scanner::new(source);
    let stream = TokenStream::new(&mut scanner);
    let mut reader = Reader::new(stream);
    reader.read_all()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(source: &str) -> Vec<SExpr> {
        read_source(source).expect("read_source should succeed")
    }

    fn one(source: &str) -> SExpr {
        let mut forms = s(source);
        assert_eq!(forms.len(), 1, "expected exactly one form, got {}", forms.len());
        forms.remove(0)
    }

    // -- atoms ------------------------------------------------------------

    #[test]
    fn atom_integer() {
        assert_eq!(s("42"), vec![SExpr::Integer(42)]);
        assert_eq!(s("0"), vec![SExpr::Integer(0)]);
    }

    #[test]
    fn atom_float() {
        assert_eq!(s("3.14"), vec![SExpr::Float(3.14)]);
    }

    #[test]
    fn atom_string() {
        assert_eq!(s(r#""hello""#), vec![SExpr::String("hello".to_string())]);
    }

    #[test]
    fn atom_symbol() {
        assert_eq!(s("foo"), vec![SExpr::Symbol("foo".to_string())]);
        assert_eq!(s("+"), vec![SExpr::Symbol("+".to_string())]);
        assert_eq!(s("defn"), vec![SExpr::Symbol("defn".to_string())]);
        assert_eq!(s("kebab-case?"), vec![SExpr::Symbol("kebab-case?".to_string())]);
        assert_eq!(s("true"), vec![SExpr::Symbol("true".to_string())]);
        assert_eq!(s("false"), vec![SExpr::Symbol("false".to_string())]);
    }

    #[test]
    fn negation_is_a_symbol_not_part_of_the_literal() {
        // The scanner (and therefore the reader) does not fuse `-` with a
        // following number — negation is parsed as a unary function call
        // in a later stage, not in the lexer/reader.
        assert_eq!(
            s("-7"),
            vec![SExpr::Symbol("-".to_string()), SExpr::Integer(7)],
        );
        assert_eq!(
            s("-2.5"),
            vec![SExpr::Symbol("-".to_string()), SExpr::Float(2.5)],
        );
    }

    #[test]
    fn special_tokens_become_symbols() {
        assert_eq!(one(":"), SExpr::Symbol(":".to_string()));
        assert_eq!(one("->"), SExpr::Symbol("->".to_string()));
        assert_eq!(one("&"), SExpr::Symbol("&".to_string()));
        assert_eq!(one("*"), SExpr::Symbol("*".to_string()));
    }

    // -- lists ------------------------------------------------------------

    #[test]
    fn empty_list_is_empty() {
        assert_eq!(s("()"), vec![SExpr::List(vec![])]);
    }

    #[test]
    fn nested_empty_lists() {
        assert_eq!(
            s("(() ())"),
            vec![SExpr::List(vec![SExpr::List(vec![]), SExpr::List(vec![])])],
        );
    }

    #[test]
    fn simple_defn_form() {
        let expr = one("(defn add [x y] (+ x y))");
        match expr {
            SExpr::List(ref items) => {
                assert_eq!(items.len(), 4);
                assert_eq!(items[0], SExpr::Symbol("defn".to_string()));
                assert_eq!(items[1], SExpr::Symbol("add".to_string()));
                assert_eq!(
                    items[2],
                    SExpr::List(vec![
                        SExpr::Symbol("x".to_string()),
                        SExpr::Symbol("y".to_string()),
                    ])
                );
                assert_eq!(
                    items[3],
                    SExpr::List(vec![
                        SExpr::Symbol("+".to_string()),
                        SExpr::Symbol("x".to_string()),
                        SExpr::Symbol("y".to_string()),
                    ])
                );
            }
            _ => panic!("expected a list"),
        }
    }

    #[test]
    fn multiple_top_level_forms() {
        let forms = s("(defn a [x] x) (defn b [y] y)");
        assert_eq!(forms.len(), 2);
    }

    #[test]
    fn if_with_optional_else() {
        // The scanner does not have a #t/#f syntax — booleans are symbols
        // resolved later by the parser.
        assert_eq!(
            one("(if true 1 2)"),
            SExpr::List(vec![
                SExpr::Symbol("if".to_string()),
                SExpr::Symbol("true".to_string()),
                SExpr::Integer(1),
                SExpr::Integer(2),
            ])
        );
        assert_eq!(
            one("(if false 0)"),
            SExpr::List(vec![
                SExpr::Symbol("if".to_string()),
                SExpr::Symbol("false".to_string()),
                SExpr::Integer(0),
            ])
        );
    }

    #[test]
    fn let_binding_with_bracket_vector() {
        let expr = one("(let [x 1 y 2] (+ x y))");
        match expr {
            SExpr::List(ref items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], SExpr::Symbol("let".to_string()));
                assert_eq!(
                    items[1],
                    SExpr::List(vec![
                        SExpr::Symbol("x".to_string()),
                        SExpr::Integer(1),
                        SExpr::Symbol("y".to_string()),
                        SExpr::Integer(2),
                    ])
                );
            }
            _ => panic!("expected a list"),
        }
    }

    #[test]
    fn brackets_are_just_lists() {
        assert_eq!(
            one("[1 2 3]"),
            SExpr::List(vec![SExpr::Integer(1), SExpr::Integer(2), SExpr::Integer(3)])
        );
        assert_eq!(
            one("{a b}"),
            SExpr::List(vec![SExpr::Symbol("a".to_string()), SExpr::Symbol("b".to_string())])
        );
    }

    #[test]
    fn deeply_nested() {
        assert_eq!(
            one("(a (b (c (d 42))))"),
            SExpr::List(vec![
                SExpr::Symbol("a".to_string()),
                SExpr::List(vec![
                    SExpr::Symbol("b".to_string()),
                    SExpr::List(vec![
                        SExpr::Symbol("c".to_string()),
                        SExpr::List(vec![SExpr::Symbol("d".to_string()), SExpr::Integer(42)]),
                    ]),
                ]),
            ])
        );
    }

    // -- comments ---------------------------------------------------------

    #[test]
    fn comments_are_stripped() {
        assert_eq!(s("; top-level comment\n42"), vec![SExpr::Integer(42)]);
        assert_eq!(
            s("(+ 1 ; inline\n2)"),
            vec![SExpr::List(vec![SExpr::Symbol("+".to_string()), SExpr::Integer(1), SExpr::Integer(2)])]
        );
    }

    // -- errors -----------------------------------------------------------

    #[test]
    fn unexpected_closer_is_an_error() {
        let err = read_source(")").unwrap_err();
        assert!(err.contains("unexpected closing"), "got: {err}");
    }

    #[test]
    fn unbalanced_open_is_an_error() {
        let err = read_source("(1 2 3").unwrap_err();
        assert!(err.contains("unexpected end of input"), "got: {err}");
    }

    #[test]
    fn empty_source_produces_no_forms() {
        assert!(read_source("").unwrap().is_empty());
        assert!(read_source("   \n  \n  ").unwrap().is_empty());
    }
}
