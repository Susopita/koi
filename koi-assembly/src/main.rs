use koi_assembly::{CompileError, compile_ir_file_to_output};

fn main() {
    if let Err(error) = run() {
        let json = serde_json::to_string_pretty(&error)
            .unwrap_or_else(|_| "{\"phase\":\"codegen\",\"severity\":\"error\",\"message\":\"failed to serialize error\",\"location\":{\"file\":\"/tmp/ir.json\",\"line\":0,\"column\":0}}".to_string());
        eprintln!("{json}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CompileError> {
    compile_ir_file_to_output("/tmp/ir.json", "output.s")
}
