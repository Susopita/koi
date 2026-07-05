use crate::frontend::ast::ASTNode;
use crate::frontend::scanner::Scanner;
use crate::frontend::token::{Token, TokenWithPos};

pub struct Parser {
    scanner: Scanner,
    current: TokenWithPos,
}

impl Parser {
    pub fn new(mut scanner: Scanner) -> Self {
        let current = scanner.next_token();
        Parser { scanner, current }
    }

    fn advance(&mut self) {
        self.current = self.scanner.next_token();
    }

    fn check(&self, token: &Token) -> bool {
        std::mem::discriminant(&self.current.token) == std::mem::discriminant(token)
    }

    fn is_keyword(&self, kw: &str) -> bool {
        matches!(&self.current.token, Token::Symbol(s) if s == kw)
    }

    fn match_token(&mut self, token: &Token) -> bool {
        if self.check(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &Token, msg: &str) -> Result<(), String> {
        if self.check(token) {
            self.advance();
            Ok(())
        } else {
            Err(format!(
                "{}. Got {:?} at line {}, column {}",
                msg, self.current.token, self.current.line, self.current.column
            ))
        }
    }

    fn expect_symbol(&mut self) -> Result<String, String> {
        match &self.current.token {
            Token::Symbol(s) => {
                let name = s.clone();
                self.advance();
                Ok(name)
            }
            other => Err(format!(
                "Expected identifier, got {:?} at line {}, column {}",
                other, self.current.line, self.current.column
            )),
        }
    }

    pub fn parse_program(&mut self) -> Result<ASTNode, String> {
        let mut children = vec![];

        while !matches!(self.current.token, Token::Eof) {
            children.push(self.parse_top_form()?);
        }

        Ok(ASTNode::Program { children })
    }

    fn parse_top_form(&mut self) -> Result<ASTNode, String> {
        if !self.check(&Token::LParen) {
            return self.parse_expr();
        }

        let line = self.current.line;
        let column = self.current.column;
        self.advance(); // consume '('

        if self.is_keyword("defn") {
            self.advance();
            self.parse_function_def(line, column)
        } else if self.is_keyword("defstruct") {
            self.advance();
            self.parse_struct_def(line, column)
        } else {
            self.parse_paren_body(line, column)
        }
    }

    /// Dispatches on the form that follows an already-consumed '('.
    /// Shared by `parse_top_form`'s fallback and `parse_expr`'s LParen case
    /// so that special forms (if/let/loop/lambda/...) work both at the top
    /// level and anywhere an expression is expected.
    fn parse_paren_body(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        if self.is_keyword("if") {
            self.advance();
            self.parse_if(line, column)
        } else if self.is_keyword("let") {
            self.advance();
            self.parse_let(line, column)
        } else if self.is_keyword("loop") {
            self.advance();
            self.parse_loop(line, column)
        } else if self.is_keyword("lambda") {
            self.advance();
            self.parse_lambda(line, column)
        } else if self.is_keyword("field") {
            self.advance();
            self.parse_field_access(line, column)
        } else if self.is_keyword("set-field!") {
            self.advance();
            self.parse_set_field(line, column)
        } else if self.is_keyword("index") {
            self.advance();
            self.parse_index(line, column)
        } else if self.is_keyword("new") {
            self.advance();
            self.parse_new(line, column)
        } else if self.is_keyword("set!") {
            self.advance();
            self.parse_set(line, column)
        } else if self.is_keyword("while") {
            self.advance();
            self.parse_while(line, column)
        } else if self.is_keyword("do") {
            self.advance();
            self.parse_do(line, column)
        } else {
            self.parse_call(line, column)
        }
    }

    fn parse_function_def(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let name = self
            .expect_symbol()
            .map_err(|e| format!("Expected function name: {}", e))?;

        self.expect(&Token::LBracket, "Expected [ to start parameter list")?;
        let parameters = self.parse_parameters()?;
        self.expect(&Token::RBracket, "Expected ] to end parameter list")?;

        let body = Box::new(self.parse_expr()?);

        self.expect(&Token::RParen, "Expected ) to close function definition")?;

        Ok(ASTNode::FunctionDef {
            name,
            parameters,
            body,
            line,
            column,
        })
    }

    fn parse_parameters(&mut self) -> Result<Vec<(String, Option<String>)>, String> {
        let mut params = vec![];

        while !self.check(&Token::RBracket) {
            let param_name = self.expect_symbol()?;

            let param_type = if self.match_token(&Token::Colon) {
                Some(self.expect_symbol()?)
            } else {
                None
            };

            params.push((param_name, param_type));
        }

        Ok(params)
    }

    fn parse_struct_def(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let name = self
            .expect_symbol()
            .map_err(|e| format!("Expected struct name: {}", e))?;

        let mut fields = vec![];
        while self.check(&Token::LBracket) {
            self.advance(); // consume '['
            let field_name = self.expect_symbol()?;
            let field_type = self.expect_symbol()?;
            self.expect(&Token::RBracket, "Expected ] to close struct field")?;
            fields.push((field_name, field_type));
        }

        self.expect(&Token::RParen, "Expected ) to close struct definition")?;

        Ok(ASTNode::StructDef {
            name,
            fields,
            line,
            column,
        })
    }

    fn parse_if(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let condition = Box::new(self.parse_expr()?);
        let then_branch = Box::new(self.parse_expr()?);

        let else_branch = if !self.check(&Token::RParen) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };

        self.expect(&Token::RParen, "Expected ) to close if expression")?;

        Ok(ASTNode::IfExpr {
            condition,
            then_branch,
            else_branch,
            line,
            column,
        })
    }

    fn parse_let(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        self.expect(&Token::LBracket, "Expected [ to start let bindings")?;

        let mut bindings = vec![];
        while !self.check(&Token::RBracket) {
            let name = self.expect_symbol()?;
            let value = Box::new(self.parse_expr()?);
            bindings.push((name, value));
        }

        self.expect(&Token::RBracket, "Expected ] to end let bindings")?;

        let body = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close let expression")?;

        Ok(ASTNode::LetBinding {
            bindings,
            body,
            line,
            column,
        })
    }

    fn parse_loop(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        self.expect(&Token::LBracket, "Expected [ to start loop binding")?;
        let variable = self.expect_symbol()?;
        let init = Box::new(self.parse_expr()?);
        self.expect(&Token::RBracket, "Expected ] to end loop binding")?;

        let condition = Box::new(self.parse_expr()?);
        let step = Box::new(self.parse_expr()?);
        let body = Box::new(self.parse_expr()?);

        self.expect(&Token::RParen, "Expected ) to close loop expression")?;

        Ok(ASTNode::LoopExpr {
            variable,
            init,
            condition,
            step,
            body,
            line,
            column,
        })
    }

    fn parse_lambda(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        self.expect(&Token::LBracket, "Expected [ to start lambda parameters")?;
        let parameters = self.parse_parameters()?;
        self.expect(&Token::RBracket, "Expected ] to end lambda parameters")?;

        let body = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close lambda expression")?;

        Ok(ASTNode::Lambda {
            parameters,
            body,
            line,
            column,
        })
    }

    fn parse_field_access(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let object = Box::new(self.parse_expr()?);
        let field = self.expect_symbol()?;
        self.expect(&Token::RParen, "Expected ) to close field access")?;

        Ok(ASTNode::FieldAccess {
            object,
            field,
            line,
            column,
        })
    }

    fn parse_set_field(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let object = Box::new(self.parse_expr()?);
        let field = self.expect_symbol()?;
        let value = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close set-field! expression")?;

        Ok(ASTNode::SetField {
            object,
            field,
            value,
            line,
            column,
        })
    }

    fn parse_index(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let array = Box::new(self.parse_expr()?);
        let index = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close index expression")?;

        Ok(ASTNode::Index {
            array,
            index,
            line,
            column,
        })
    }

    fn parse_new(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let type_str = self.expect_symbol()?;

        let size_or_init = if !self.check(&Token::RParen) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };

        self.expect(&Token::RParen, "Expected ) to close new expression")?;

        Ok(ASTNode::New {
            type_str,
            size_or_init,
            line,
            column,
        })
    }

    fn parse_set(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let name = self
            .expect_symbol()
            .map_err(|e| format!("Expected variable name to set!: {}", e))?;
        let value = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close set! expression")?;

        Ok(ASTNode::SetVar {
            name,
            value,
            line,
            column,
        })
    }

    fn parse_while(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let condition = Box::new(self.parse_expr()?);
        let body = Box::new(self.parse_expr()?);
        self.expect(&Token::RParen, "Expected ) to close while expression")?;

        Ok(ASTNode::WhileExpr {
            condition,
            body,
            line,
            column,
        })
    }

    fn parse_do(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let mut exprs = vec![];

        while !self.check(&Token::RParen) && !matches!(self.current.token, Token::Eof) {
            exprs.push(self.parse_expr()?);
        }

        if exprs.is_empty() {
            return Err(format!(
                "do requires at least one expression. Got {:?} at line {}, column {}",
                self.current.token, self.current.line, self.current.column
            ));
        }

        self.expect(&Token::RParen, "Expected ) to close do expression")?;

        Ok(ASTNode::DoExpr {
            exprs,
            line,
            column,
        })
    }

    fn parse_expr(&mut self) -> Result<ASTNode, String> {
        match &self.current.token {
            Token::IntLiteral(n) => {
                let val = *n;
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "int64".to_string(),
                    value: serde_json::json!(val),
                    line,
                    column,
                })
            }
            Token::FloatLiteral(f) => {
                let val = *f;
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "float64".to_string(),
                    value: serde_json::json!(val),
                    line,
                    column,
                })
            }
            Token::StringLiteral(s) => {
                let val = s.clone();
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "string".to_string(),
                    value: serde_json::json!(val),
                    line,
                    column,
                })
            }
            Token::BoolLiteral(b) => {
                let val = *b;
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "bool".to_string(),
                    value: serde_json::json!(val),
                    line,
                    column,
                })
            }
            Token::Symbol(s) if s == "true" => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "bool".to_string(),
                    value: serde_json::json!(true),
                    line,
                    column,
                })
            }
            Token::Symbol(s) if s == "false" => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Literal {
                    literal_type: "bool".to_string(),
                    value: serde_json::json!(false),
                    line,
                    column,
                })
            }
            Token::Symbol(s) => {
                let name = s.clone();
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                Ok(ASTNode::Variable { name, line, column })
            }
            Token::Ampersand => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                let operand = Box::new(self.parse_expr()?);
                Ok(ASTNode::AddrOf {
                    operand,
                    line,
                    column,
                })
            }
            Token::Asterisk => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                let operand = Box::new(self.parse_expr()?);
                Ok(ASTNode::Deref {
                    operand,
                    line,
                    column,
                })
            }
            Token::LParen => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                self.parse_paren_body(line, column)
            }
            Token::LBracket => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                let elements = self.parse_array_elements()?;
                self.expect(&Token::RBracket, "Expected ] to close array literal")?;
                Ok(ASTNode::ArrayLiteral {
                    elements,
                    line,
                    column,
                })
            }
            other => Err(format!(
                "Unexpected token: {:?} at line {}, column {}",
                other, self.current.line, self.current.column
            )),
        }
    }

    fn parse_call(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        // A bare '*' in function position means multiplication, e.g. `(* a b)` --
        // not "dereference the rest of the call". Everywhere else '*' is a
        // dereference prefix (see parse_expr), so this has to be special-cased
        // here rather than in parse_expr.
        let function = Box::new(if self.check(&Token::Asterisk) {
            let l = self.current.line;
            let c = self.current.column;
            self.advance();
            ASTNode::Variable {
                name: "*".to_string(),
                line: l,
                column: c,
            }
        } else {
            self.parse_expr()?
        });

        let mut arguments = vec![];
        while !self.check(&Token::RParen) && !matches!(self.current.token, Token::Eof) {
            arguments.push(self.parse_expr()?);
        }

        self.expect(&Token::RParen, "Expected ) to close call")?;

        Ok(ASTNode::Call {
            function,
            arguments,
            line,
            column,
        })
    }

    fn parse_array_elements(&mut self) -> Result<Vec<ASTNode>, String> {
        let mut elements = vec![];

        while !self.check(&Token::RBracket) && !matches!(self.current.token, Token::Eof) {
            elements.push(self.parse_expr()?);
        }

        Ok(elements)
    }
}
