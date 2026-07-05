use std::path::PathBuf;

use clap::Parser;

use koi::pipeline::{BuildError, BuildMode};

#[derive(Parser)]
#[command(
    name = "koi",
    about = "Koi compiler for the Carp language",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Compile a Carp source file
    Build {
        /// Path to the .carp source file
        file: PathBuf,

        /// Stop after parsing/scope analysis and print the AST to stdout
        #[arg(long, conflicts_with = "check")]
        dump_ast: bool,

        /// Run type-checking only (no code generation)
        #[arg(long, conflicts_with = "dump_ast")]
        check: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Build {
            file,
            dump_ast,
            check,
        } => {
            let input = match std::fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[io] Could not read '{}': {e}", file.display());
                    std::process::exit(1);
                }
            };

            let mode = if *dump_ast {
                BuildMode::DumpAst
            } else if *check {
                BuildMode::Check
            } else {
                BuildMode::Full
            };

            match run_build(&input, file, mode) {
                Ok(Some(ast)) => {
                    // --dump-ast: print AST as pretty JSON to stdout
                    let json =
                        serde_json::to_string_pretty(&ast).expect("AST serialization should not fail");
                    println!("{json}");
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

fn run_build(
    input: &str,
    src_path: &PathBuf,
    mode: BuildMode,
) -> Result<Option<koi::frontend::ast::ASTNode>, BuildError> {
    // Stage 1: Frontend (always)
    let ast = koi::pipeline::run_frontend(input)?;

    if mode == BuildMode::DumpAst {
        return Ok(Some(ast));
    }

    // Stage 2: Middle-end
    let ir = koi::pipeline::run_middle_end(&ast)?;

    if mode == BuildMode::Check {
        eprintln!("[check] No errors found.");
        return Ok(None);
    }

    // Stage 3: Backend
    koi::pipeline::run_backend(ir, src_path)?;

    eprintln!("[koi] Build complete: output.s + executable '{}'", 
        src_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output"));

    Ok(None)
}
