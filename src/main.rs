use std::path::PathBuf;

use clap::Parser;

use koi::pipeline::{BuildMode, PipelineResult};

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

        /// (IDE mode) Only parse and print structured JSON to stdout —
        /// no code generation, no human-readable messages.
        #[arg(long, conflicts_with = "check")]
        dump_ast: bool,

        /// (IDE mode) Type-check only — emit structured JSON to stdout,
        /// no code generation.
        #[arg(long, conflicts_with = "dump_ast")]
        check: bool,

        /// Pretty-print the JSON output (default in IDE modes is compact).
        #[arg(long)]
        pretty: bool,

        /// Skip borrow-checking — useful for testing / debugging.
        #[arg(long)]
        no_borrow_check: bool,

        /// Target architecture (x86_64, arm64, riscv).
        /// Defaults to x86_64.
        #[arg(long, default_value = "x86_64")]
        target: String,
    },
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Build {
            file,
            dump_ast,
            check,
            pretty,
            target,
            no_borrow_check,
        } => {
            let arch = match koi::backend::TargetArch::from_str(target) {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };

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

            let (result, diag) = koi::pipeline::run_pipeline(&input, file, mode, arch, *no_borrow_check);

            match mode {
                BuildMode::DumpAst | BuildMode::Check => {
                    if let PipelineResult::Json(json) = &result {
                        if *pretty {
                            if let Ok(value) =
                                serde_json::from_str::<serde_json::Value>(json)
                            {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&value).unwrap()
                                );
                            }
                        } else {
                            println!("{json}");
                        }
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    }
                    if diag.has_errors() {
                        std::process::exit(1);
                    }
                }
                BuildMode::Full => {
                    let had_errors = diag.has_errors();
                    for d in &diag.diagnostics {
                        let loc = d
                            .location
                            .as_ref()
                            .map(|l| format!(":{}:{}", l.line, l.column))
                            .unwrap_or_default();
                        if d.severity == "error" {
                            eprintln!("[{}]{loc} {}", d.phase, d.message);
                        } else {
                            eprintln!("[{}]{loc} warning: {}", d.phase, d.message);
                        }
                    }

                    if !had_errors {
                        let exe_name = file
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("output");
                        eprintln!(
                            "[koi] Build complete: output.s + executable '{exe_name}'"
                        );
                    } else {
                        std::process::exit(1);
                    }
                }
            }
        }
    }
}
