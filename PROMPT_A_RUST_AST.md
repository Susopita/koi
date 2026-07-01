# **🦀 PROMPT A: RUST CRATE AST — Lexer, Parser, AST, Scope**

**Responsabilidad:** Implementar análisis léxico y sintáctico para S-expressions Carp en **Rust**, generar AST con enums, y resolver alcance.

**Entrada:** `test.carp` (archivo de texto)

**Salida:** `/tmp/ast.json` (AST serializado)

**Timeline:** Días 1-4 de 1 semana

**Crate:** `koi-ast` (binario)

---

## **Parte 1: Tokenización (Lexer) — Rust Idiomático**

### **Token Enum (Rust)**

```rust
// koi-ast/src/token.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Token {
    // Delimitadores
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    
    // Literales
    IntLiteral(i64),
    FloatLiteral(f64),
    BoolLiteral(bool),
    StringLiteral(String),
    
    // Símbolos e identificadores
    Symbol(String),  // +, -, *, /, foo, defn, etc.
    
    // Keywords (como parte de Symbol, se validan en parser)
    // Palabras clave: defn, defstruct, lambda, let, if, loop, do, new, etc.
    
    // Especiales
    Colon,
    Arrow,    // ->
    Ampersand, // &
    Asterisk,  // *
    
    // Control
    Eof,
}

#[derive(Debug, Clone)]
pub struct TokenWithPos {
    pub token: Token,
    pub line: usize,
    pub column: usize,
}
```

### **Scanner (Lexer)**

```rust
// koi-ast/src/scanner.rs

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
        if self.pos < self.input.len() {
            Some(self.input[self.pos])
        } else {
            None
        }
    }
    
    fn peek(&self, offset: usize) -> Option<char> {
        let pos = self.pos + offset;
        if pos < self.input.len() {
            Some(self.input[pos])
        } else {
            None
        }
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
        // ; comentario hasta EOL
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
            if ch.is_numeric() {
                num_str.push(ch);
                self.advance();
            } else if ch == '.' && !is_float && self.peek(1).map_or(false, |c| c.is_numeric()) {
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
                        _ => escaped,
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
        
        // Símbolos especiales de 1-2 caracteres: +, -, *, /, ==, !=, etc.
        match self.current() {
            Some('+') | Some('-') | Some('*') | Some('/') => {
                symbol.push(self.current().unwrap());
                self.advance();
                
                // Chequear dos caracteres
                if let Some(ch) = self.current() {
                    if (symbol == "=" || symbol == "!" || symbol == "<" || symbol == ">")
                        && ch == '=' {
                        symbol.push(ch);
                        self.advance();
                    } else if symbol == "*" && ch == '*' {
                        symbol.push(ch);
                        self.advance();
                    } else if symbol == "&" && ch == '&' {
                        symbol.push(ch);
                        self.advance();
                    } else if symbol == "|" && ch == '|' {
                        symbol.push(ch);
                        self.advance();
                    }
                }
            }
            Some('<') | Some('>') | Some('!') | Some('=') | Some('&') | Some('|') => {
                symbol.push(self.current().unwrap());
                self.advance();
                
                if let Some(ch) = self.current() {
                    if ch == '=' || (symbol == "&" && ch == '&') || (symbol == "|" && ch == '|') {
                        symbol.push(ch);
                        self.advance();
                    }
                }
            }
            _ => {
                // Identificador: [a-zA-Z_][a-zA-Z0-9_?!]*
                while let Some(ch) = self.current() {
                    if ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!' {
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
                None => return TokenWithPos { token: Token::Eof, line, column },
                Some(';') => {
                    self.skip_comment();
                    continue;
                }
                Some('(') => {
                    self.advance();
                    return TokenWithPos { token: Token::LParen, line, column };
                }
                Some(')') => {
                    self.advance();
                    return TokenWithPos { token: Token::RParen, line, column };
                }
                Some('[') => {
                    self.advance();
                    return TokenWithPos { token: Token::LBracket, line, column };
                }
                Some(']') => {
                    self.advance();
                    return TokenWithPos { token: Token::RBracket, line, column };
                }
                Some('{') => {
                    self.advance();
                    return TokenWithPos { token: Token::LBrace, line, column };
                }
                Some('}') => {
                    self.advance();
                    return TokenWithPos { token: Token::RBrace, line, column };
                }
                Some(':') => {
                    self.advance();
                    return TokenWithPos { token: Token::Colon, line, column };
                }
                Some('&') if self.peek(1) != Some('&') => {
                    self.advance();
                    return TokenWithPos { token: Token::Ampersand, line, column };
                }
                Some('"') => {
                    let token = self.read_string();
                    return TokenWithPos { token, line, column };
                }
                Some(ch) if ch.is_numeric() => {
                    let token = self.read_number();
                    return TokenWithPos { token, line, column };
                }
                Some(ch) if ch.is_alphabetic() || ch == '_' => {
                    let token = self.read_symbol();
                    return TokenWithPos { token, line, column };
                }
                Some(ch) if "+-*/<>=!&|".contains(ch) => {
                    let token = self.read_symbol();
                    return TokenWithPos { token, line, column };
                }
                Some(_) => {
                    self.advance();
                    // Skip unknown character, or emit error token
                    continue;
                }
            }
        }
    }
}
```

---

## **Parte 2: AST con Enums + Serde**

### **AST Definition (Rust)**

```rust
// koi-ast/src/ast.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "nodeType")]
pub enum ASTNode {
    #[serde(rename = "program")]
    Program {
        children: Vec<ASTNode>,
    },
    
    #[serde(rename = "function_def")]
    FunctionDef {
        name: String,
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "struct_def")]
    StructDef {
        name: String,
        fields: Vec<(String, String)>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "call")]
    Call {
        function: Box<ASTNode>,
        arguments: Vec<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "variable")]
    Variable {
        name: String,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "literal")]
    Literal {
        #[serde(rename = "literalType")]
        literal_type: String, // "int64", "float64", "bool", "string"
        value: serde_json::Value,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "lambda")]
    Lambda {
        parameters: Vec<(String, Option<String>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "let_binding")]
    LetBinding {
        bindings: Vec<(String, Box<ASTNode>)>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "if")]
    IfExpr {
        condition: Box<ASTNode>,
        then_branch: Box<ASTNode>,
        else_branch: Option<Box<ASTNode>>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "loop")]
    LoopExpr {
        variable: String,
        init: Box<ASTNode>,
        condition: Box<ASTNode>,
        step: Box<ASTNode>,
        body: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "field_access")]
    FieldAccess {
        object: Box<ASTNode>,
        field: String,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "index")]
    Index {
        array: Box<ASTNode>,
        index: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "addr_of")]
    AddrOf {
        operand: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "deref")]
    Deref {
        operand: Box<ASTNode>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "new")]
    New {
        type_str: String,
        size_or_init: Option<Box<ASTNode>>,
        line: usize,
        column: usize,
    },
    
    #[serde(rename = "array_literal")]
    ArrayLiteral {
        elements: Vec<ASTNode>,
        line: usize,
        column: usize,
    },
}

impl ASTNode {
    pub fn line(&self) -> usize {
        match self {
            ASTNode::FunctionDef { line, .. } => *line,
            ASTNode::StructDef { line, .. } => *line,
            ASTNode::Call { line, .. } => *line,
            ASTNode::Variable { line, .. } => *line,
            ASTNode::Literal { line, .. } => *line,
            ASTNode::Lambda { line, .. } => *line,
            ASTNode::LetBinding { line, .. } => *line,
            ASTNode::IfExpr { line, .. } => *line,
            ASTNode::LoopExpr { line, .. } => *line,
            ASTNode::FieldAccess { line, .. } => *line,
            ASTNode::Index { line, .. } => *line,
            ASTNode::AddrOf { line, .. } => *line,
            ASTNode::Deref { line, .. } => *line,
            ASTNode::New { line, .. } => *line,
            ASTNode::ArrayLiteral { line, .. } => *line,
            ASTNode::Program { .. } => 0,
        }
    }
    
    pub fn column(&self) -> usize {
        match self {
            ASTNode::FunctionDef { column, .. } => *column,
            ASTNode::StructDef { column, .. } => *column,
            ASTNode::Call { column, .. } => *column,
            ASTNode::Variable { column, .. } => *column,
            ASTNode::Literal { column, .. } => *column,
            ASTNode::Lambda { column, .. } => *column,
            ASTNode::LetBinding { column, .. } => *column,
            ASTNode::IfExpr { column, .. } => *column,
            ASTNode::LoopExpr { column, .. } => *column,
            ASTNode::FieldAccess { column, .. } => *column,
            ASTNode::Index { column, .. } => *column,
            ASTNode::AddrOf { column, .. } => *column,
            ASTNode::Deref { column, .. } => *column,
            ASTNode::New { column, .. } => *column,
            ASTNode::ArrayLiteral { column, .. } => *column,
            ASTNode::Program { .. } => 0,
        }
    }
}
```

---

## **Parte 3: Parser Recursive Descent (Rust)**

```rust
// koi-ast/src/parser.rs

use crate::ast::ASTNode;
use crate::scanner::Scanner;
use crate::token::{Token, TokenWithPos};

pub struct Parser {
    scanner: Scanner,
    current: TokenWithPos,
    errors: Vec<String>,
}

impl Parser {
    pub fn new(mut scanner: Scanner) -> Self {
        let current = scanner.next_token();
        Parser {
            scanner,
            current,
            errors: vec![],
        }
    }
    
    fn advance(&mut self) {
        self.current = self.scanner.next_token();
    }
    
    fn check(&self, token: &Token) -> bool {
        std::mem::discriminant(&self.current.token) == std::mem::discriminant(token)
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
            Err(format!("{}. Got {:?} at line {}", msg, self.current.token, self.current.line))
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
        match &self.current.token {
            Token::LParen => {
                self.advance();
                match &self.current.token {
                    Token::Symbol(s) if s == "defn" => {
                        self.advance();
                        self.parse_function_def()
                    }
                    Token::Symbol(s) if s == "defstruct" => {
                        self.advance();
                        self.parse_struct_def()
                    }
                    _ => {
                        // Es una expresión en top level
                        self.pos -= 1; // "Retroceder"
                        self.parse_expr()
                    }
                }
            }
            _ => self.parse_expr(),
        }
    }
    
    fn parse_function_def(&mut self) -> Result<ASTNode, String> {
        let name = match &self.current.token {
            Token::Symbol(s) => {
                let name = s.clone();
                self.advance();
                name
            }
            _ => return Err("Expected function name".to_string()),
        };
        
        self.expect(&Token::LBracket, "Expected [")?;
        let parameters = self.parse_parameters()?;
        self.expect(&Token::RBracket, "Expected ]")?;
        
        let body = Box::new(self.parse_expr()?);
        
        self.expect(&Token::RParen, "Expected ) to close function definition")?;
        
        Ok(ASTNode::FunctionDef {
            name,
            parameters,
            body,
            line: self.current.line,
            column: self.current.column,
        })
    }
    
    fn parse_parameters(&mut self) -> Result<Vec<(String, Option<String>)>, String> {
        let mut params = vec![];
        
        while !self.check(&Token::RBracket) {
            let param_name = match &self.current.token {
                Token::Symbol(s) => {
                    let name = s.clone();
                    self.advance();
                    name
                }
                _ => return Err("Expected parameter name".to_string()),
            };
            
            let param_type = if self.match_token(&Token::Colon) {
                // Optional type annotation
                match &self.current.token {
                    Token::Symbol(s) => {
                        let ty = s.clone();
                        self.advance();
                        Some(ty)
                    }
                    _ => return Err("Expected type after :".to_string()),
                }
            } else {
                None
            };
            
            params.push((param_name, param_type));
        }
        
        Ok(params)
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
            Token::LParen => {
                let line = self.current.line;
                let column = self.current.column;
                self.advance();
                self.parse_call(line, column)
            }
            Token::LBracket => {
                self.advance();
                let elements = self.parse_array_elements()?;
                self.expect(&Token::RBracket, "Expected ]")?;
                Ok(ASTNode::ArrayLiteral {
                    elements,
                    line: self.current.line,
                    column: self.current.column,
                })
            }
            _ => Err(format!("Unexpected token: {:?}", self.current.token)),
        }
    }
    
    fn parse_call(&mut self, line: usize, column: usize) -> Result<ASTNode, String> {
        let function = Box::new(self.parse_expr()?);
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
```

---

## **Parte 4: Scope Analysis (Rust)**

```rust
// koi-ast/src/scope.rs

use std::collections::HashMap;
use crate::ast::ASTNode;

pub struct ScopeAnalyzer {
    scopes: Vec<HashMap<String, String>>,
    functions: std::collections::HashSet<String>,
    errors: Vec<String>,
}

impl ScopeAnalyzer {
    pub fn new() -> Self {
        ScopeAnalyzer {
            scopes: vec![HashMap::new()],
            functions: std::collections::HashSet::new(),
            errors: vec![],
        }
    }
    
    pub fn analyze(&mut self, node: &ASTNode) -> Result<(), Vec<String>> {
        self.analyze_node(node);
        
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors.clone())
        }
    }
    
    fn analyze_node(&mut self, node: &ASTNode) {
        match node {
            ASTNode::Program { children } => {
                for child in children {
                    self.analyze_node(child);
                }
            }
            ASTNode::FunctionDef { name, parameters, body, .. } => {
                self.functions.insert(name.clone());
                
                // Nueva scope para función
                let mut scope = HashMap::new();
                for (param_name, _) in parameters {
                    scope.insert(param_name.clone(), "param".to_string());
                }
                self.scopes.push(scope);
                
                self.analyze_node(body);
                
                self.scopes.pop();
            }
            ASTNode::Variable { name, line, column } => {
                if !self.is_declared(name) && !self.functions.contains(name) {
                    self.errors.push(format!(
                        "Variable '{}' not declared at line {}, column {}",
                        name, line, column
                    ));
                }
            }
            ASTNode::Call { function, arguments, .. } => {
                self.analyze_node(function);
                for arg in arguments {
                    self.analyze_node(arg);
                }
            }
            ASTNode::LetBinding { bindings, body, .. } => {
                // Nueva scope para let
                let mut scope = HashMap::new();
                for (var_name, value) in bindings {
                    self.analyze_node(value);
                    scope.insert(var_name.clone(), "local".to_string());
                }
                self.scopes.push(scope);
                self.analyze_node(body);
                self.scopes.pop();
            }
            ASTNode::IfExpr {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.analyze_node(condition);
                self.analyze_node(then_branch);
                if let Some(els) = else_branch {
                    self.analyze_node(els);
                }
            }
            // ... more cases
            _ => {}
        }
    }
    
    fn is_declared(&self, name: &str) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.contains_key(name) {
                return true;
            }
        }
        false
    }
}
```

---

## **Parte 5: Main Entry Point (Rust)**

```rust
// koi-ast/src/main.rs

mod token;
mod scanner;
mod parser;
mod ast;
mod scope;

use std::fs;
use std::io::{self, Read};
use scanner::Scanner;
use parser::Parser;
use scope::ScopeAnalyzer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() != 2 {
        eprintln!("Usage: {} <file.carp>", args[0]);
        std::process::exit(1);
    }
    
    let filename = &args[1];
    let input = fs::read_to_string(filename)?;
    
    // Lexing + Parsing
    let scanner = Scanner::new(&input);
    let mut parser = Parser::new(scanner);
    
    let program = match parser.parse_program() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            std::process::exit(1);
        }
    };
    
    // Scope Analysis
    let mut scope_analyzer = ScopeAnalyzer::new();
    if let Err(errors) = scope_analyzer.analyze(&program) {
        for err in errors {
            eprintln!("Scope error: {}", err);
        }
        std::process::exit(1);
    }
    
    // Serialize to JSON
    let json = serde_json::to_string_pretty(&program)?;
    
    // Write to /tmp/ast.json
    fs::write("/tmp/ast.json", json)?;
    
    println!("✓ AST complete. AST saved to /tmp/ast.json");
    
    Ok(())
}
```

---

## **Cargo.toml para koi-ast**

```toml
[package]
name = "koi-ast"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[[bin]]
name = "koi-ast"
path = "src/main.rs"
```

---

## **Tests**

**test_simple.carp:**
```lisp
(defn add [x y]
  (+ x y))

(defn main []
  (print (add 5 3))
  0)
```

**Esperado en /tmp/ast.json:**
```json
{
  "nodeType": "program",
  "children": [
    {
      "nodeType": "function_def",
      "name": "add",
      "parameters": [
        ["x", null],
        ["y", null]
      ],
      "body": {
        "nodeType": "call",
        "function": {
          "nodeType": "variable",
          "name": "+"
        },
        "arguments": [
          {"nodeType": "variable", "name": "x"},
          {"nodeType": "variable", "name": "y"}
        ]
      },
      ...
    }
  ]
}
```

---

## **Build & Run**

```bash
# En koi-ast/
cargo build --release

# Ejecutar frontend
./target/release/koi-ast test.carp

# Validar salida
cat /tmp/ast.json | jq .
```

---

## **Checklist AST koi - Rust (4 días)**

- [ ] Día 1: Token enum + Scanner
- [ ] Día 2: Parser recursive descent
- [ ] Día 3: AST enums + serde serialization
- [ ] Día 4: Scope analyzer + main.rs
- [ ] Validar /tmp/ast.json válido

¡Listos para construir el Koi-AST en Rust! 🦀
