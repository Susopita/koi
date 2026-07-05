use crate::frontend::token::{Token, TokenWithPos};

pub struct Scanner {
    input: Vec<char>,
    pos: usize,
    line: usize,
    column: usize,
}

impl Scanner {
    pub fn new(input: &str) -> Self {
        Scanner {
            input: input.chars().collect(),
            pos: 0,
            line: 1,
            column: 1,
        }
    }

    fn current(&self) -> Option<char> {
        self.input.get(self.pos).copied()
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.input.get(self.pos + offset).copied()
    }

    fn advance(&mut self) {
        if let Some(ch) = self.current() {
            if ch == '\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
            self.pos += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.current() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn skip_comment(&mut self) {
        // ; comment until end of line
        if self.current() == Some(';') {
            while let Some(ch) = self.current() {
                if ch == '\n' {
                    break;
                }
                self.advance();
            }
        }
    }

    fn read_number(&mut self) -> Token {
        let mut num_str = String::new();
        let mut is_float = false;

        while let Some(ch) = self.current() {
            if ch.is_ascii_digit() {
                num_str.push(ch);
                self.advance();
            } else if ch == '.' && !is_float && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                num_str.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if is_float {
            Token::FloatLiteral(num_str.parse().unwrap_or(0.0))
        } else {
            Token::IntLiteral(num_str.parse().unwrap_or(0))
        }
    }

    fn read_string(&mut self) -> Token {
        self.advance(); // consume opening "
        let mut string = String::new();

        while let Some(ch) = self.current() {
            if ch == '"' {
                self.advance(); // consume closing "
                break;
            } else if ch == '\\' {
                self.advance();
                if let Some(escaped) = self.current() {
                    string.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        other => other,
                    });
                    self.advance();
                }
            } else {
                string.push(ch);
                self.advance();
            }
        }

        Token::StringLiteral(string)
    }

    fn read_symbol(&mut self) -> Token {
        let mut symbol = String::new();

        match self.current() {
            Some('+') | Some('-') | Some('*') | Some('/') => {
                symbol.push(self.current().unwrap());
                self.advance();

                // '*' doubles up into "**" (exponentiation); +, -, / never combine.
                if self.current() == Some('*') && symbol == "*" {
                    symbol.push('*');
                    self.advance();
                }
            }
            Some('<') | Some('>') | Some('!') | Some('=') | Some('&') | Some('|') => {
                symbol.push(self.current().unwrap());
                self.advance();

                if let Some(ch) = self.current()
                    && (ch == '=' || (symbol == "&" && ch == '&') || (symbol == "|" && ch == '|'))
                {
                    symbol.push(ch);
                    self.advance();
                }
            }
            _ => {
                // Identifier: [a-zA-Z_][a-zA-Z0-9_?!-]*
                // '-' is allowed *inside/after* the first character so kebab-case
                // names (e.g. `apply-func`, `make-origin`) lex as one symbol; a
                // leading '-' is still handled by the operator branch above so
                // `(- x 1)` keeps working.
                while let Some(ch) = self.current() {
                    if ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!' || ch == '-' {
                        symbol.push(ch);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
        }

        Token::Symbol(symbol)
    }

    pub fn next_token(&mut self) -> TokenWithPos {
        loop {
            self.skip_whitespace();

            let line = self.line;
            let column = self.column;

            match self.current() {
                None => {
                    return TokenWithPos {
                        token: Token::Eof,
                        line,
                        column,
                    };
                }
                Some(';') => {
                    self.skip_comment();
                    continue;
                }
                Some('(') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::LParen,
                        line,
                        column,
                    };
                }
                Some(')') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::RParen,
                        line,
                        column,
                    };
                }
                Some('[') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::LBracket,
                        line,
                        column,
                    };
                }
                Some(']') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::RBracket,
                        line,
                        column,
                    };
                }
                Some('{') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::LBrace,
                        line,
                        column,
                    };
                }
                Some('}') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::RBrace,
                        line,
                        column,
                    };
                }
                Some(':') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::Colon,
                        line,
                        column,
                    };
                }
                Some('-') if self.peek(1) == Some('>') => {
                    self.advance();
                    self.advance();
                    return TokenWithPos {
                        token: Token::Arrow,
                        line,
                        column,
                    };
                }
                Some('&') if self.peek(1) != Some('&') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::Ampersand,
                        line,
                        column,
                    };
                }
                Some('*') if self.peek(1) != Some('*') => {
                    self.advance();
                    return TokenWithPos {
                        token: Token::Asterisk,
                        line,
                        column,
                    };
                }
                Some('"') => {
                    let token = self.read_string();
                    return TokenWithPos {
                        token,
                        line,
                        column,
                    };
                }
                Some(ch) if ch.is_ascii_digit() => {
                    let token = self.read_number();
                    return TokenWithPos {
                        token,
                        line,
                        column,
                    };
                }
                Some(ch) if ch.is_alphabetic() || ch == '_' => {
                    let token = self.read_symbol();
                    return TokenWithPos {
                        token,
                        line,
                        column,
                    };
                }
                Some(ch) if "+-*/<>=!&|".contains(ch) => {
                    let token = self.read_symbol();
                    return TokenWithPos {
                        token,
                        line,
                        column,
                    };
                }
                Some(_) => {
                    // Skip unrecognized character.
                    self.advance();
                    continue;
                }
            }
        }
    }
}
