//! Interactive REPL (Read-Eval-Print Loop)
//!
//! VBC-backed evaluation. Each input is parsed; top-level items
//! (`fn`, `type`, `protocol`, `implement`, `static`, `mount`) are
//! accumulated into a session source buffer. Bare expressions are
//! wrapped in a one-shot `__repl_main_<n>()` function that prints
//! the result; the session buffer is recompiled with the new function
//! and that function is executed via the VBC interpreter.
//!
//! `let NAME [: TYPE] = EXPR` at the REPL surface is desugared to a
//! `static NAME[: TYPE] = EXPR;` so bindings persist across prompts.
//! `:reset` clears the session.

use anyhow::Result;
use colored::Colorize;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;
use tracing::debug;
use verum_ast::FileId;
use verum_common::Text;
use verum_lexer::Lexer;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

use crate::session::Session;

/// Interactive REPL backed by the VBC interpreter.
pub struct Repl {
    session: Session,
    line_number: usize,
    multiline_buffer: Text,
    /// Accumulated top-level source. Each prompt that introduces
    /// a top-level item or `static` appends to this buffer; the
    /// buffer is recompiled on every evaluation.
    session_source: String,
    /// Monotonic counter used to give each `__repl_main_N` a unique name.
    eval_counter: usize,
}

impl Repl {
    /// Create a new REPL instance
    pub fn new(session: Session) -> Self {
        Self {
            session,
            line_number: 1,
            multiline_buffer: Text::new(),
            session_source: String::new(),
            eval_counter: 0,
        }
    }

    /// Preload a module by appending its source to the session buffer.
    pub fn preload(&mut self, path: &Path) -> Result<()> {
        let file_id = self.session.load_file(path)?;
        let source = self
            .session
            .get_source(file_id)
            .ok_or_else(|| anyhow::anyhow!("Failed to load preload file"))?;

        // Validate the preload by compiling it standalone first; only
        // commit to the session buffer on success.
        compile_module_str(&source.source).map_err(|e| anyhow::anyhow!(e))?;

        self.session_source.push_str(&source.source);
        if !self.session_source.ends_with('\n') {
            self.session_source.push('\n');
        }

        println!("{} Preloaded: {}", "✓".green(), path.display());

        Ok(())
    }

    /// Run the REPL
    pub fn run(&mut self) -> Result<()> {
        self.print_welcome();

        loop {
            let prompt = if self.multiline_buffer.is_empty() {
                format!("verum[{}]> ", self.line_number)
            } else {
                "       ... ".to_string()
            };

            print!("{}", prompt.cyan());
            io::stdout().flush()?;

            let mut input = String::new();
            if io::stdin().read_line(&mut input)? == 0 {
                println!();
                break;
            }

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
                    self.session_source.clear();
                    self.eval_counter = 0;
                    self.multiline_buffer.clear();
                    println!("REPL reset");
                    continue;
                }
                ":source" => {
                    self.print_session_source();
                    continue;
                }
                ":status" => {
                    self.print_status();
                    continue;
                }
                _ => {}
            }

            self.multiline_buffer.push_str(&input);

            if self.is_complete(&self.multiline_buffer) {
                let buffer_to_eval = self.multiline_buffer.clone();
                let result = self.evaluate(&buffer_to_eval);
                self.multiline_buffer.clear();

                match result {
                    Ok(output) => {
                        if !output.is_empty() {
                            print!("{}", output);
                            if !output.as_str().ends_with('\n') {
                                println!();
                            }
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
        println!("Type :help for help, :quit to exit");
        println!();
    }

    /// Print help message
    fn print_help(&self) {
        println!("{}", "\nREPL Commands:".bold());
        println!("  :help, :h       Show this help");
        println!("  :quit, :q       Exit REPL");
        println!("  :clear, :c      Clear multiline buffer");
        println!("  :reset, :r      Reset REPL state");
        println!("  :source         Show accumulated session source");
        println!("  :status         Show pipeline status");
        println!();
        println!("{}", "Examples:".bold());
        println!("  1 + 2 * 3                       (evaluated and printed)");
        println!("  let x = 42                      (becomes a session-level static)");
        println!("  x + 10                          (prints 52)");
        println!("  fn add(a: Int, b: Int) -> Int {{ a + b }}");
        println!("  add(2, 3)                       (prints 5)");
        println!();
    }

    fn print_status(&self) {
        println!();
        println!("{}", "REPL Pipeline".bold());
        println!("{}", "═".repeat(45).cyan());
        println!();
        println!("{} Parsing (verum_fast_parser)", "✓".green());
        println!("{} VBC codegen (verum_vbc::codegen)", "✓".green());
        println!("{} VBC interpreter (verum_vbc::interpreter)", "✓".green());
        println!("{} Persistent session source ({} chars)", "✓".green(), self.session_source.len());
        println!();
    }

    fn print_session_source(&self) {
        if self.session_source.is_empty() {
            println!("{}", "Session source is empty.".dimmed());
        } else {
            println!("{}", "Session source:".bold());
            println!("{}", self.session_source);
        }
    }

    /// Check if input is complete (balanced braces).
    pub fn is_complete(&self, input: &str) -> bool {
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

    /// Evaluate input via the VBC interpreter.
    fn evaluate(&mut self, input: &str) -> Result<Text> {
        debug!("Evaluating: {}", input);

        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(Text::new());
        }

        let file_id = FileId::new(u32::MAX - self.line_number as u32);
        let parser = verum_fast_parser::VerumParser::new();

        // 1. `let NAME [: TYPE] = EXPR` → desugar to a session-level
        //    declaration. With a type annotation, emit `static NAME:
        //    TYPE = EXPR;` (mutation across prompts isn't supported,
        //    but `static` accepts the broader value forms a user
        //    typically wants in a REPL). Without a type, fall back
        //    to `const` which permits inference.
        if let Some((name, ty_opt, expr_src)) = parse_repl_let(trimmed) {
            let stmt = match ty_opt {
                Some(ty) => format!("static {}: {} = {};\n", name, ty, expr_src),
                None => format!("const {} = {};\n", name, expr_src),
            };
            self.try_compile_appended(&stmt)?;
            self.session_source.push_str(&stmt);
            return Ok(format!("{} bound {}", "✓".green(), name.cyan()).into());
        }

        // 2. Top-level items (fn/type/protocol/impl/static/mount).
        // Try parsing as a module first — this catches multi-item input too.
        let lexer = Lexer::new(trimmed, file_id);
        if let Ok(module) = parser.parse_module(lexer, file_id)
            && !module.items.is_empty()
            && module.items.iter().all(is_top_level_item)
        {
            let appended = format!("{}\n", trimmed);
            self.try_compile_appended(&appended)?;
            self.session_source.push_str(&appended);
            let count = module.items.len();
            return Ok(format!("{} {} item(s) defined", "✓".green(), count).into());
        }

        // 2b. Script-mode top-level statements: let-bindings, defer,
        // expression-statements, sequences thereof. Parses through
        // the same `parse_module_script_str` that drives `verum
        // hello.vr`, so what works at the REPL prompt also works in
        // a script file. The synthesised `__verum_script_main`
        // wrapper signals that real statements were present (a
        // pure-decl input would have been caught by branch 2 above).
        // The input is wrapped in a fresh REPL function and
        // executed; captured stdout is returned. State persistence
        // across snippets (so `let x = 1` in one prompt visible to
        // the next) is REPL-state infrastructure — landed
        // separately.
        let lexer = Lexer::new(trimmed, file_id);
        let _ = lexer; // re-create below; the previous lexer was consumed
        if let Ok(module) = parser.parse_module_script_str(trimmed, file_id) {
            let has_wrapper = module.items.iter().any(|i| {
                if let verum_ast::ItemKind::Function(f) = &i.kind {
                    f.name.as_str() == "__verum_script_main"
                } else {
                    false
                }
            });
            if has_wrapper {
                self.eval_counter += 1;
                let func_name = format!("__repl_script_{}", self.eval_counter);
                let wrapper = format!(
                    "fn {}() {{\n    {}\n}}\n",
                    func_name, trimmed
                );
                let stdout = self.compile_and_run(&wrapper, &func_name)?;
                return Ok(stdout.into());
            }
        }

        // 3. Bare expression → wrap in __repl_main_N() and execute.
        if parser.parse_expr_str(trimmed, file_id).is_ok() {
            self.eval_counter += 1;
            let func_name = format!("__repl_main_{}", self.eval_counter);
            // f-string interpolation prints the value; works for any
            // type that has a Display/Debug surface in stdlib's print path.
            let wrapper = format!(
                "fn {}() {{\n    print(f\"{{{}}}\");\n}}\n",
                func_name, trimmed
            );
            let stdout = self.compile_and_run(&wrapper, &func_name)?;
            return Ok(stdout.into());
        }

        Err(anyhow::anyhow!("Could not parse input"))
    }

    /// Verify that `state.session_source ++ fragment` still compiles.
    fn try_compile_appended(&self, fragment: &str) -> Result<()> {
        let mut source = self.session_source.clone();
        source.push_str(fragment);
        if !source.contains("fn main") {
            source.push_str("\nfn main() {}\n");
        }
        compile_module_str(&source).map(|_| ()).map_err(|e| anyhow::anyhow!(e))
    }

    /// Compile `session_source ++ extra_source`, locate `func_name`,
    /// execute it via the VBC interpreter, and return captured stdout.
    fn compile_and_run(&self, extra_source: &str, func_name: &str) -> Result<String> {
        let mut source = self.session_source.clone();
        source.push_str(extra_source);
        if !source.contains("fn main") {
            source.push_str("\nfn main() {}\n");
        }
        let vbc_module = compile_module_str(&source).map_err(|e| anyhow::anyhow!(e))?;
        let vbc_module = Arc::new(vbc_module);
        let func_id = vbc_module
            .functions
            .iter()
            .find(|f| vbc_module.get_string(f.name) == Some(func_name))
            .map(|f| f.id)
            .ok_or_else(|| anyhow::anyhow!("internal: REPL wrapper {} not found", func_name))?;

        let mut interpreter = Interpreter::new(vbc_module);
        interpreter
            .execute_function(func_id)
            .map_err(|e| anyhow::anyhow!("runtime error: {:?}", e))?;
        Ok(interpreter.state.get_stdout().to_string())
    }
}

/// Compile a source string through the same path as `verum run`.
fn compile_module_str(source: &str) -> std::result::Result<verum_vbc::VbcModule, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = verum_fast_parser::VerumParser::new();
    let module = parser
        .parse_module(lexer, file_id)
        .map_err(|errs| {
            let first = errs
                .iter()
                .next()
                .map(|e| format!("{:?}", e))
                .unwrap_or_default();
            format!("parse error: {}", first)
        })?;
    let config = CodegenConfig::new("repl");
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module(&module)
        .map_err(|e| format!("codegen error: {:?}", e))
}

fn parse_repl_let(input: &str) -> Option<(&str, Option<&str>, &str)> {
    let trimmed = input.trim().trim_end_matches(';').trim();
    let rest = trimmed.strip_prefix("let ")?;
    let rest = rest.strip_prefix("mut ").unwrap_or(rest);
    let eq = rest.find('=')?;
    let lhs = rest[..eq].trim();
    let expr = rest[eq + 1..].trim();
    if lhs.is_empty() || expr.is_empty() {
        return None;
    }
    if let Some(colon) = lhs.find(':') {
        let name = lhs[..colon].trim();
        let ty = lhs[colon + 1..].trim();
        if !is_simple_ident(name) || ty.is_empty() {
            return None;
        }
        Some((name, Some(ty), expr))
    } else {
        if !is_simple_ident(lhs) {
            return None;
        }
        Some((lhs, None, expr))
    }
}

fn is_simple_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn is_top_level_item(item: &verum_ast::Item) -> bool {
    use verum_ast::ItemKind;
    matches!(
        &item.kind,
        ItemKind::Function(_)
            | ItemKind::Type(_)
            | ItemKind::Protocol(_)
            | ItemKind::Impl(_)
            | ItemKind::Static(_)
            | ItemKind::Const(_)
    )
}
