//! Interactive REPL (Read-Eval-Print Loop)
//!
//! # VBC-First Architecture Migration
//!
//! The compiler REPL is being migrated to VBC-first architecture.
//! Currently supports parsing and type checking only.
//! For execution, use `verum run` with AOT compilation.

use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};
use std::path::Path;
use tracing::debug;
use verum_ast::FileId;
use verum_common::Text;
use verum_lexer::Lexer;
use verum_types::TypeChecker;

use crate::session::Session;

/// Interactive REPL (parse and type check only during VBC migration)
pub struct Repl {
    session: Session,
    checker: TypeChecker,
    line_number: usize,
    multiline_buffer: Text,
    /// Show types instead of values
    _show_types: bool,
}

impl Repl {
    /// Create a new REPL instance
    pub fn new(session: Session) -> Self {
        Self {
            session,
            checker: TypeChecker::new(),
            line_number: 1,
            multiline_buffer: Text::new(),
            _show_types: true, // Default to showing types during migration
        }
    }

    /// Preload a module
    pub fn preload(&mut self, path: &Path) -> Result<()> {
        let file_id = self.session.load_file(path)?;
        let source = self
            .session
            .get_source(file_id)
            .ok_or_else(|| anyhow::anyhow!("Failed to load preload file"))?;

        // Parse and type check
        let lexer = Lexer::new(&source.source, file_id);
        let parser = verum_fast_parser::VerumParser::new();
        let module = parser
            .parse_module(lexer, file_id)
            .map_err(|errors| anyhow::anyhow!("Parse errors: {:?}", errors))?;

        // Type check
        for item in &module.items {
            if let Err(e) = self.checker.check_item(item) {
                println!("{}: {}", "warning".yellow(), e);
            }
        }

        println!("{} Preloaded: {}", "✓".green(), path.display());

        Ok(())
    }

    /// Run the REPL
    pub fn run(&mut self) -> Result<()> {
        self.print_welcome();

        loop {
            // Print prompt
            let prompt = if self.multiline_buffer.is_empty() {
                format!("verum[{}]> ", self.line_number)
            } else {
                "       ... ".to_string()
            };

            print!("{}", prompt.cyan());
            io::stdout().flush()?;

            // Read line
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            // Handle special commands
            let trimmed = input.trim();
            if trimmed.is_empty() {
                continue;
            }

            match trimmed {
                ":quit" | ":q" | ":exit" => {
                    println!("Goodbye!");
                    break;
                }
                ":help" | ":h" => {
                    self.print_help();
                    continue;
                }
                ":clear" | ":c" => {
                    self.multiline_buffer.clear();
                    println!("Buffer cleared");
                    continue;
                }
                ":reset" | ":r" => {
                    self.checker = TypeChecker::new();
                    self.multiline_buffer.clear();
                    println!("REPL reset");
                    continue;
                }
                ":status" => {
                    self.print_status();
                    continue;
                }
                _ => {}
            }

            // Accumulate input
            self.multiline_buffer.push_str(&input);

            // Try to evaluate
            let is_complete = self.is_complete(&self.multiline_buffer);
            if is_complete {
                let buffer_to_eval = self.multiline_buffer.clone();
                let result = self.evaluate(&buffer_to_eval);
                self.multiline_buffer.clear();

                match result {
                    Ok(output) => {
                        if !output.is_empty() {
                            println!("{}", output);
                        }
                    }
                    Err(e) => {
                        println!("{}: {}", "error".red(), e);
                    }
                }

                self.line_number += 1;
            }
        }

        Ok(())
    }

    /// Print welcome message
    fn print_welcome(&self) {
        println!("{}", "Verum REPL".bold());
        println!("Version: {}", env!("CARGO_PKG_VERSION"));
        println!();
        println!(
            "{}",
            "Note: REPL is in type-check mode during VBC migration.".yellow()
        );
        println!(
            "{}",
            "Expressions are parsed and type-checked but not evaluated.".yellow()
        );
        println!();
        println!("Type :help for help, :quit to exit");
        println!();
    }

    /// Print help message
    fn print_help(&self) {
        println!("{}", "\nREPL Commands:".bold());
        println!("  :help, :h       Show this help");
        println!("  :quit, :q       Exit REPL");
        println!("  :clear, :c      Clear multiline buffer");
        println!("  :reset, :r      Reset type checker state");
        println!("  :status         Show VBC migration status");
        println!();
        println!("{}", "VBC Migration:".bold().yellow());
        println!("  The REPL is being migrated to VBC-first architecture.");
        println!("  Expressions are parsed and type-checked, but not executed.");
        println!("  For full execution, use: verum run --mode aot <file.vr>");
        println!();
        println!("{}", "Examples (type checking):".bold());
        println!("  1 + 2 * 3                   => Int");
        println!("  let x = 42                  => binds x : Int");
        println!("  fn add(a: Int, b: Int) -> Int {{ a + b }}");
        println!();
    }

    /// Print VBC migration status
    fn print_status(&self) {
        println!();
        println!("{}", "VBC-First Architecture Migration Status".bold());
        println!("{}", "═".repeat(45).cyan());
        println!();
        println!("{} Parsing (verum_fast_parser)", "✓".green());
        println!("{} Type checking (verum_types)", "✓".green());
        println!("{} VBC codegen (verum_vbc::codegen)", "◐".yellow());
        println!("{} VBC interpreter (verum_vbc::interpreter)", "◐".yellow());
        println!("{} REPL execution", "◯".dimmed());
        println!();
        println!("{}", "For full execution:".bold());
        println!("  verum run --mode aot <file.vr>");
        println!();
    }

    /// Check if input is complete
    pub fn is_complete(&self, input: &str) -> bool {
        // Simple heuristic: check balanced braces
        let mut depth = 0;
        for ch in input.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        depth == 0
    }

    /// Evaluate input (parse and type-check only during VBC migration)
    fn evaluate(&mut self, input: &str) -> Result<Text> {
        debug!("Evaluating: {}", input);

        // Create temporary file ID for REPL input
        let file_id = FileId::new(u32::MAX - self.line_number as u32);

        let parser = verum_fast_parser::VerumParser::new();

        // Try to parse as expression first
        if let Ok(expr) = parser.parse_expr_str(input, file_id) {
            // Type check expression
            match self.checker.synth_expr(&expr) {
                Ok(result) => {
                    return Ok(format!(
                        "{} : {}",
                        "expr".dimmed(),
                        result.ty.to_string().cyan()
                    )
                    .into());
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Type error: {}", e));
                }
            }
        }

        // Try to parse as a module (which could contain statements/items)
        let lexer = Lexer::new(input, file_id);
        if let Ok(module) = parser.parse_module(lexer, file_id) {
            if module.items.is_empty() {
                return Ok(Text::new());
            }

            // Type check all items
            let mut defined_count = 0;
            for item in &module.items {
                if let Err(e) = self.checker.check_item(item) {
                    return Err(anyhow::anyhow!("Type error: {}", e));
                }
                defined_count += 1;
            }

            return Ok(format!("{} {} item(s) defined", "✓".green(), defined_count).into());
        }

        Err(anyhow::anyhow!("Could not parse input"))
    }
}
