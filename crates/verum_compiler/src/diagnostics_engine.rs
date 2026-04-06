//! Unified Diagnostics Engine
//!
//! Provides rich error messages with:
//! - Source code snippets
//! - Colorized output
//! - Suggestions and fixes
//! - Error recovery
//!
//! Diagnostic infrastructure: structured error/warning/info messages with
//! source spans, suggestions, and fix-it hints. Supports LSP integration.

use colored::Colorize;
use std::fmt;
use verum_diagnostics::{Diagnostic, Severity};
use verum_common::{List, Text};

/// Unified diagnostics engine
pub struct DiagnosticsEngine {
    diagnostics: List<Diagnostic>,
    error_count: usize,
    warning_count: usize,
}

impl DiagnosticsEngine {
    pub fn new() -> Self {
        Self {
            diagnostics: List::new(),
            error_count: 0,
            warning_count: 0,
        }
    }

    /// Emit a diagnostic
    pub fn emit(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Error => self.error_count += 1,
            Severity::Warning => self.warning_count += 1,
            _ => {}
        }

        self.diagnostics.push(diagnostic);
    }

    /// Get all diagnostics
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Has errors?
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Get error count
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Get warning count
    pub fn warning_count(&self) -> usize {
        self.warning_count
    }

    /// Clear all diagnostics
    pub fn clear(&mut self) {
        self.diagnostics.clear();
        self.error_count = 0;
        self.warning_count = 0;
    }

    /// Print all diagnostics
    pub fn print_all(&self) {
        for diagnostic in &self.diagnostics {
            self.print_diagnostic(diagnostic);
        }

        if self.has_errors() {
            eprintln!(
                "\n{}: {} error(s), {} warning(s)",
                "Compilation failed".red().bold(),
                self.error_count,
                self.warning_count
            );
        }
    }

    /// Print a single diagnostic
    fn print_diagnostic(&self, diagnostic: &Diagnostic) {
        let severity_str = match diagnostic.severity() {
            Severity::Error => "error".red().bold(),
            Severity::Warning => "warning".yellow().bold(),
            Severity::Note => "note".cyan(),
            Severity::Help => "help".blue().bold(),
        };

        eprintln!("{}: {}", severity_str, diagnostic.message());

        for help in diagnostic.helps() {
            eprintln!("  {}: {}", "help".cyan(), help);
        }

        for note in diagnostic.notes() {
            eprintln!("  {}: {}", "note".cyan(), note);
        }
    }

    /// Generate summary
    pub fn summary(&self) -> Text {
        format!(
            "{} error(s), {} warning(s)",
            self.error_count, self.warning_count
        )
        .into()
    }
}

impl Default for DiagnosticsEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DiagnosticsEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}
