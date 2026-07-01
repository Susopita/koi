use koi_ast::parser::Parser;
use koi_ast::scanner::Scanner;
use koi_ast::scope::ScopeAnalyzer;
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <file.carp>", args[0]);
        std::process::exit(1);
    }

    let filename = &args[1];

    let input = match fs::read_to_string(filename) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("[lexer] Could not read file '{}': {}", filename, e);
            std::process::exit(1);
        }
    };

    let scanner = Scanner::new(&input);
    let mut parser = Parser::new(scanner);

    let program = match parser.parse_program() {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("[parser] {}", e);
            std::process::exit(1);
        }
    };

    let mut scope_analyzer = ScopeAnalyzer::new();
    if let Err(errors) = scope_analyzer.analyze(&program) {
        for err in errors {
            eprintln!("[scope] {}", err);
        }
        std::process::exit(1);
    }

    let json = match serde_json::to_string_pretty(&program) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("[ast] Failed to serialize AST: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = fs::write("/tmp/ast.json", json) {
        eprintln!("[ast] Failed to write /tmp/ast.json: {}", e);
        std::process::exit(1);
    }

    println!("AST complete. Saved to /tmp/ast.json");
}
