//! Structured diagnostic output for the IDE / language-server protocol.
//!
//! Every diagnostic follows a uniform schema so that the Pond IDE can parse
//! it without scraping human-readable stderr.  The CLI emits this JSON
//! **exclusively to stdout** when `--check` or `--dump-ast` is used, and
//! the program exits with code 0 if the check passed or 1 if it didn't.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Source location
// ---------------------------------------------------------------------------

/// A precise location in the source file.
#[derive(Debug, Clone, Serialize)]
pub struct SourceLocation {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

// ---------------------------------------------------------------------------
// Diagnostic entry
// ---------------------------------------------------------------------------

/// A single diagnostic emitted by any phase of the compiler.
#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    /// Which compiler phase produced this diagnostic
    /// (`"lexer"`, `"parser"`, `"scope"`, `"inference"`, `"borrow_check"`).
    pub phase: String,
    /// `"error"` or `"warning"`.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
    /// Precise source location, if available.
    pub location: Option<SourceLocation>,
}

impl Diagnostic {
    pub fn error(phase: impl Into<String>, message: impl Into<String>) -> Self {
        Diagnostic {
            phase: phase.into(),
            severity: "error".to_string(),
            message: message.into(),
            location: None,
        }
    }

    pub fn with_location(mut self, file: &str, line: usize, column: usize) -> Self {
        self.location = Some(SourceLocation {
            file: file.to_string(),
            line,
            column,
        });
        self
    }
}

// ---------------------------------------------------------------------------
// Top-level output envelopes
// ---------------------------------------------------------------------------

/// JSON envelope for `koi build --check <file>`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckOutput {
    pub success: bool,
    pub diagnostics: Vec<Diagnostic>,
}

/// JSON envelope for `koi build --dump-ast <file>`.
#[derive(Debug, Clone, Serialize)]
pub struct DumpAstOutput {
    pub ast: serde_json::Value,
}

/// JSON envelope for the full pipeline (assembly written to disk).
#[derive(Debug, Clone, Serialize)]
pub struct BuildOutput {
    pub success: bool,
    pub assembly_path: String,
    pub executable_path: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

// ---------------------------------------------------------------------------
// Accumulator (used internally by the pipeline before final emission)
// ---------------------------------------------------------------------------

/// Accumulates diagnostics across phases and can produce final JSON.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticBag {
    pub diagnostics: Vec<Diagnostic>,
    pub file: String,
}

impl DiagnosticBag {
    pub fn new(file: &str) -> Self {
        DiagnosticBag {
            diagnostics: Vec::new(),
            file: file.to_string(),
        }
    }

    pub fn push(&mut self, phase: &str, severity: &str, message: String) {
        self.diagnostics.push(Diagnostic {
            phase: phase.to_string(),
            severity: severity.to_string(),
            message,
            location: None,
        });
    }

    pub fn push_with_location(
        &mut self,
        phase: &str,
        severity: &str,
        message: String,
        line: usize,
        column: usize,
    ) {
        self.diagnostics.push(Diagnostic {
            phase: phase.to_string(),
            severity: severity.to_string(),
            message,
            location: Some(SourceLocation {
                file: self.file.clone(),
                line,
                column,
            }),
        });
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == "error")
    }

    /// Serialise to a JSON string for stdout.
    pub fn to_json(&self) -> String {
        let output = CheckOutput {
            success: !self.has_errors(),
            diagnostics: self.diagnostics.clone(),
        };
        serde_json::to_string_pretty(&output).unwrap_or_else(|_| {
            r#"{"success":false,"diagnostics":[{"phase":"io","severity":"error","message":"serialization failed"}]}"#.to_string()
        })
    }
}
