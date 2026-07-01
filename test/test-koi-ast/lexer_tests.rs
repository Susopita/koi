use koi_ast::scanner::Scanner;
use koi_ast::token::Token;

fn tokens(src: &str) -> Vec<Token> {
    let mut scanner = Scanner::new(src);
    let mut out = vec![];
    loop {
        let tok = scanner.next_token();
        let is_eof = matches!(tok.token, Token::Eof);
        out.push(tok.token);
        if is_eof {
            break;
        }
    }
    out
}

#[test]
fn delimiters() {
    assert_eq!(
        tokens("()[]{}"),
        vec![
            Token::LParen,
            Token::RParen,
            Token::LBracket,
            Token::RBracket,
            Token::LBrace,
            Token::RBrace,
            Token::Eof,
        ]
    );
}

#[test]
fn int_literal() {
    assert_eq!(tokens("42"), vec![Token::IntLiteral(42), Token::Eof]);
}

#[test]
fn float_literal() {
    assert_eq!(tokens("2.5"), vec![Token::FloatLiteral(2.5), Token::Eof]);
}

#[test]
fn negative_number_lexes_as_minus_then_int() {
    // There is no fused negative-literal syntax: `-5` is the unary/binary
    // minus symbol followed by `5`. `(- 5)` / `(- 0 5)` express negation.
    assert_eq!(
        tokens("-5"),
        vec![Token::Symbol("-".into()), Token::IntLiteral(5), Token::Eof]
    );
}

#[test]
fn string_literal_with_escapes() {
    let src = r#""a\nb\t\"c\"""#;
    assert_eq!(
        tokens(src),
        vec![Token::StringLiteral("a\nb\t\"c\"".into()), Token::Eof]
    );
}

#[test]
fn comment_is_skipped_entirely() {
    assert_eq!(
        tokens("; a full line comment\n42"),
        vec![Token::IntLiteral(42), Token::Eof]
    );
}

#[test]
fn comment_without_trailing_newline_at_eof() {
    assert_eq!(tokens("; trailing comment"), vec![Token::Eof]);
}

#[test]
fn kebab_case_identifier_is_one_symbol() {
    // Regression: the original scanner didn't allow '-' inside identifiers,
    // so "apply-func" lexed as three tokens (apply, -, func).
    assert_eq!(
        tokens("apply-func"),
        vec![Token::Symbol("apply-func".into()), Token::Eof]
    );
}

#[test]
fn leading_minus_before_identifier_is_still_binary_op() {
    // Because a leading '-' takes the operator-start path (not the
    // identifier-start path), `(- x 1)` must not be swallowed into one symbol.
    assert_eq!(
        tokens("(- x 1)"),
        vec![
            Token::LParen,
            Token::Symbol("-".into()),
            Token::Symbol("x".into()),
            Token::IntLiteral(1),
            Token::RParen,
            Token::Eof,
        ]
    );
}

#[test]
fn identifiers_with_question_and_bang() {
    assert_eq!(
        tokens("empty? set!"),
        vec![
            Token::Symbol("empty?".into()),
            Token::Symbol("set!".into()),
            Token::Eof,
        ]
    );
}

#[test]
fn two_char_comparison_and_logical_operators() {
    assert_eq!(
        tokens("<= >= == != && ||"),
        vec![
            Token::Symbol("<=".into()),
            Token::Symbol(">=".into()),
            Token::Symbol("==".into()),
            Token::Symbol("!=".into()),
            Token::Symbol("&&".into()),
            Token::Symbol("||".into()),
            Token::Eof,
        ]
    );
}

#[test]
fn single_vs_double_ampersand() {
    assert_eq!(
        tokens("& &&"),
        vec![Token::Ampersand, Token::Symbol("&&".into()), Token::Eof]
    );
}

#[test]
fn single_vs_double_asterisk() {
    assert_eq!(
        tokens("* **"),
        vec![Token::Asterisk, Token::Symbol("**".into()), Token::Eof]
    );
}

#[test]
fn colon_token() {
    assert_eq!(tokens(":"), vec![Token::Colon, Token::Eof]);
}

#[test]
fn arrow_token() {
    assert_eq!(tokens("->"), vec![Token::Arrow, Token::Eof]);
}

#[test]
fn unrecognized_character_is_skipped() {
    // Documents current behavior: stray bytes outside the language's
    // character set are dropped rather than raising a lexer error.
    assert_eq!(tokens("@42"), vec![Token::IntLiteral(42), Token::Eof]);
}

#[test]
fn empty_input_is_just_eof() {
    assert_eq!(tokens(""), vec![Token::Eof]);
}

#[test]
fn line_and_column_tracking_across_newlines() {
    let mut scanner = Scanner::new("(add\n  x\n  y)");
    let lparen = scanner.next_token();
    assert_eq!(lparen.line, 1);
    assert_eq!(lparen.column, 1);

    let add = scanner.next_token();
    assert_eq!(add.token, Token::Symbol("add".into()));
    assert_eq!(add.line, 1);
    assert_eq!(add.column, 2);

    let x = scanner.next_token();
    assert_eq!(x.token, Token::Symbol("x".into()));
    assert_eq!(x.line, 2);
    assert_eq!(x.column, 3);

    let y = scanner.next_token();
    assert_eq!(y.token, Token::Symbol("y".into()));
    assert_eq!(y.line, 3);
    assert_eq!(y.column, 3);
}
