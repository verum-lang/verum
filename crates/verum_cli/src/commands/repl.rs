//! REPL command - Interactive Read-Eval-Print Loop for Verum
//!
//! VBC-backed evaluation. Each input is parsed; top-level items
//! (`fn`, `type`, `protocol`, `implement`, `static`, `mount`) are
//! accumulated into a session source buffer. Bare expressions are
//! wrapped in a one-shot `__repl_main_<n>()` function that prints
//! the result, the session buffer is recompiled with the new
//! function, and the function is executed via the VBC interpreter.
//!
//! Each `let`-style binding is desugared at the REPL surface into a
//! `static` declaration so it persists across prompts. `:reset`
//! clears the session.

use colored::Colorize;
use std::io::{self, Write};
use verum_common::{List, Map, Text};

use crate::error::Result;
use crate::ui;

use std::sync::Arc;
use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::interpreter::Interpreter;

/// REPL state tracking (VBC-first version)
struct ReplState {
    parser: VerumParser,
    history: List<Text>,
    /// Accumulated top-level source (items and statics).
    /// Concatenated with each new input before compilation.
    session_source: String,
    /// Names declared so far → kind label, shown by `:bindings`.
    bindings: Map<Text, Text>,
    line_number: usize,
    multiline_buffer: Text,
    in_multiline: bool,
    /// Monotonically increasing counter for unique `__repl_main_N`.
    eval_counter: usize,
}

impl ReplState {
    fn new() -> Result<Self> {
        Ok(Self {
            parser: VerumParser::new(),
            history: List::new(),
            session_source: String::new(),
            bindings: Map::new(),
            line_number: 1,
            multiline_buffer: Text::new(),
            in_multiline: false,
            eval_counter: 0,
        })
    }

    fn add_to_history(&mut self, input: &str) {
        self.history.push(input.into());
    }
}

/// Execute the REPL command
pub fn execute(_tier: Option<u8>) -> Result<()> {
    print_welcome();

    let mut state = ReplState::new()?;

    loop {
        print_prompt(&state);

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            println!();
            break;
        }

        let input = input.trim();

        if input.starts_with(':') {
            if !handle_command(input, &mut state)? {
                break;
            }
            continue;
        }

        if input.ends_with('{') || state.in_multiline {
            state.multiline_buffer.push_str(input);
            state.multiline_buffer.push('\n');

            let open_braces = state.multiline_buffer.as_str().matches('{').count();
            let close_braces = state.multiline_buffer.as_str().matches('}').count();

            if open_braces > close_braces {
                state.in_multiline = true;
                continue;
            } else {
                state.in_multiline = false;
                let complete_input = state.multiline_buffer.clone();
                state.multiline_buffer.clear();
                process_input(complete_input.as_str(), &mut state);
            }
        } else if !input.is_empty() {
            process_input(input, &mut state);
        }

        state.line_number += 1;
    }

    println!("{}", "Goodbye!".cyan());
    Ok(())
}

fn print_welcome() {
    println!();
    println!(
        "{}",
        "═══════════════════════════════════════════════".cyan()
    );
    println!(
        "{} {} {}",
        "Welcome to".bold(),
        "Verum REPL".cyan().bold(),
        format!("v{}", env!("CARGO_PKG_VERSION")).dimmed()
    );
    println!(
        "{}",
        "═══════════════════════════════════════════════".cyan()
    );
    println!();
    println!(
        "Type {} for help, {} to exit",
        ":help".cyan(),
        ":quit".cyan()
    );
    println!();
}

fn print_prompt(state: &ReplState) {
    if state.in_multiline {
        print!(
            "{}{}  ",
            "...".dimmed(),
            " ".repeat(format!("{}", state.line_number).len())
        );
    } else {
        print!(
            "{} {} ",
            format!("[{}]", state.line_number).dimmed(),
            "verum>".cyan().bold()
        );
    }
    io::stdout().flush().unwrap();
}

fn handle_command(command: &str, state: &mut ReplState) -> Result<bool> {
    let parts: List<&str> = command.split_whitespace().collect();
    let cmd = parts.first().unwrap_or(&"");

    match *cmd {
        ":quit" | ":q" | ":exit" => {
            return Ok(false);
        }
        ":help" | ":h" | ":?" => {
            print_help();
        }
        ":clear" | ":cls" => {
            print!("\x1B[2J\x1B[1;1H");
            print_welcome();
        }
        ":bindings" | ":b" => {
            print_bindings(state);
        }
        ":history" | ":hist" => {
            print_history(state);
        }
        ":reset" => {
            *state = ReplState::new()?;
            ui::success("Environment reset");
        }
        ":source" => {
            print_session_source(state);
        }
        ":ast" => {
            if parts.len() < 2 {
                ui::error("Usage: :ast <expression>");
            } else {
                let expr: String = parts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                show_ast(&expr, state);
            }
        }
        _ => {
            ui::error(&format!("Unknown command: {}", cmd));
            ui::info("Type :help for available commands");
        }
    }

    Ok(true)
}

fn print_help() {
    println!();
    println!("{}", "REPL Commands:".bold());
    println!("  {}     Quit the REPL", ":quit, :q".cyan());
    println!("  {}        Show this help", ":help, :h".cyan());
    println!("  {}       Clear screen", ":clear".cyan());
    println!("  {} <expr>     Show AST of expression", ":ast".cyan());
    println!("  {}    Show all bindings", ":bindings".cyan());
    println!("  {}     Show command history", ":history".cyan());
    println!("  {}      Show accumulated session source", ":source".cyan());
    println!("  {}       Reset environment", ":reset".cyan());
    println!();
    println!("{}", "Examples:".bold());
    println!("  let x = 42                       (becomes a session-level static)");
    println!("  x + 10                           (evaluated and printed)");
    println!("  fn add(a: Int, b: Int) -> Int {{ a + b }}");
    println!("  add(2, 3)                        (prints 5)");
    println!();
}

fn process_input(input: &str, state: &mut ReplState) {
    state.add_to_history(input);

    // 1. `let NAME = EXPR` (with optional `: TYPE`) → desugar to
    //    `static NAME: TYPE = EXPR;` so the binding persists across prompts.
    if let Some((name, ty_opt, expr_src)) = parse_repl_let(input) {
        let stmt = match ty_opt {
            Some(ty) => format!("static {}: {} = {};\n", name, ty, expr_src),
            None => format!("static {} = {};\n", name, expr_src),
        };
        if try_compile_with_appended(state, &stmt).is_ok() {
            state.session_source.push_str(&stmt);
            state.bindings.insert(Text::from(name), Text::from("static"));
            println!(
                "{} {} bound {}",
                format!("[{}]", state.line_number).dimmed(),
                "✓".green(),
                name.cyan()
            );
        }
        return;
    }

    // 2. Top-level item parse (fn, type, protocol, implement, static, mount).
    let file_id = FileId::new(state.line_number as u32);
    let lexer = Lexer::new(input, file_id);
    let tokens: List<verum_lexer::Token> = lexer.filter_map(|r| r.ok()).collect();
    let tokens_list: List<_> = tokens.iter().cloned().collect();
    if let Ok(item) = state.parser.parse_item_tokens(&tokens_list) {
        let appended = format!("{}\n", input);
        if try_compile_with_appended(state, &appended).is_err() {
            return;
        }
        state.session_source.push_str(&appended);
        let (kind, name) = item_kind_and_name(&item);
        if let Some(n) = name {
            state.bindings.insert(Text::from(n.clone()), Text::from(kind));
            println!(
                "{} {} {} {}",
                format!("[{}]", state.line_number).dimmed(),
                "✓".green(),
                kind,
                n.cyan()
            );
        } else {
            println!(
                "{} {} {}",
                format!("[{}]", state.line_number).dimmed(),
                "✓".green(),
                kind
            );
        }
        return;
    }

    // 3. Bare expression → wrap in __repl_main_N() that prints the result.
    if state.parser.parse_expr_str(input, file_id).is_ok() {
        state.eval_counter += 1;
        let func_name = format!("__repl_main_{}", state.eval_counter);
        let wrapper = format!(
            "fn {}() {{\n    print(f\"{{{}}}\");\n}}\n",
            func_name, input
        );
        match compile_and_run(state, &wrapper, &func_name) {
            Ok(stdout) => {
                if !stdout.is_empty() {
                    print!(
                        "{} {}",
                        format!("[{}]", state.line_number).dimmed(),
                        stdout
                    );
                    if !stdout.ends_with('\n') {
                        println!();
                    }
                }
            }
            Err(msg) => {
                ui::error(&msg);
            }
        }
        return;
    }

    ui::error("Failed to parse input — not a let, item, or expression");
    ui::info("Try :help to see examples");
}

/// Parse a `let NAME [: TYPE] = EXPR` form at the REPL surface.
/// Returns `(name, type, expr_src)` on match. The expression is
/// captured verbatim so the parser can validate it during compile.
fn parse_repl_let(input: &str) -> Option<(&str, Option<&str>, &str)> {
    let trimmed = input.trim().trim_end_matches(';').trim();
    let rest = trimmed.strip_prefix("let ")?;
    // mut prefix is allowed — we still desugar to a static for now;
    // mutation across prompts would need richer state.
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
        if name.is_empty() || ty.is_empty() || !is_simple_ident(name) {
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

fn item_kind_and_name(item: &verum_ast::Item) -> (&'static str, Option<String>) {
    use verum_ast::ItemKind;
    match &item.kind {
        ItemKind::Function(f) => ("fn", Some(f.name.name.to_string())),
        ItemKind::Type(t) => ("type", Some(t.name.name.to_string())),
        ItemKind::Protocol(p) => ("protocol", Some(p.name.name.to_string())),
        ItemKind::Static(s) => ("static", Some(s.name.name.to_string())),
        ItemKind::Const(c) => ("const", Some(c.name.name.to_string())),
        ItemKind::Impl(_) => ("implement", None),
        _ => ("item", None),
    }
}

/// Ensure the new fragment compiles when appended to the session source.
/// Returns Ok if the resulting module compiles cleanly.
fn try_compile_with_appended(state: &ReplState, fragment: &str) -> Result<()> {
    let mut source = state.session_source.clone();
    source.push_str(fragment);
    // Append a no-op main to satisfy the entry-point lookup; the
    // compile-only check doesn't actually run it.
    if !source.contains("fn main") {
        source.push_str("\nfn main() {}\n");
    }
    compile_module(&source).map(|_| ()).map_err(|e| {
        ui::error(&e);
        crate::error::CliError::Custom(e)
    })
}

/// Compile `state.session_source ++ extra_source`, locate `func_name`,
/// execute it, and return the captured stdout.
fn compile_and_run(
    state: &ReplState,
    extra_source: &str,
    func_name: &str,
) -> std::result::Result<String, String> {
    let mut source = state.session_source.clone();
    source.push_str(extra_source);
    if !source.contains("fn main") {
        source.push_str("\nfn main() {}\n");
    }
    let vbc_module = compile_module(&source)?;
    let vbc_module = Arc::new(vbc_module);
    let func_id = vbc_module
        .functions
        .iter()
        .find(|f| vbc_module.get_string(f.name) == Some(func_name))
        .map(|f| f.id)
        .ok_or_else(|| format!("internal: REPL wrapper {} not found", func_name))?;

    let mut interpreter = Interpreter::new(vbc_module);
    interpreter
        .execute_function(func_id)
        .map_err(|e| format!("runtime error: {:?}", e))?;
    Ok(interpreter.state.get_stdout().to_string())
}

/// Compile a source string to a VBC module via the same path as
/// `verum run` (single-file interpreter mode). Returns a brief
/// error message on parse / typecheck / codegen failure.
fn compile_module(source: &str) -> std::result::Result<verum_vbc::VbcModule, String> {
    let file_id = FileId::new(0);
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();
    let module = parser
        .parse_module(lexer, file_id)
        .map_err(|errs| {
            let first = errs.iter().next().map(|e| format!("{:?}", e)).unwrap_or_default();
            format!("parse error: {}", first)
        })?;

    let config = CodegenConfig::new("repl");
    let mut codegen = VbcCodegen::with_config(config);
    codegen
        .compile_module(&module)
        .map_err(|e| format!("codegen error: {:?}", e))
}

fn show_ast(expr: &str, state: &ReplState) {
    let file_id = FileId::new(999999);

    match state.parser.parse_expr_str(expr, file_id) {
        Ok(parsed_expr) => {
            println!("{}", "AST:".bold());
            println!("{:#?}", parsed_expr);
        }
        Err(errors) => {
            for error in errors {
                ui::error(&format!("Parse error: {}", error));
            }
        }
    }
}

fn print_session_source(state: &ReplState) {
    if state.session_source.is_empty() {
        ui::info("Session source is empty");
    } else {
        println!("{}", "Session source:".bold());
        println!("{}", state.session_source);
    }
}

fn print_bindings(state: &ReplState) {
    if state.bindings.is_empty() {
        ui::info("No bindings defined");
    } else {
        println!("{}", "Current bindings:".bold());
        for (name, kind) in &state.bindings {
            println!("  {} : {}", name.as_str().cyan(), kind);
        }
    }
}

fn print_history(state: &ReplState) {
    if state.history.is_empty() {
        ui::info("No history");
    } else {
        println!("{}", "Command history:".bold());
        for (i, cmd) in state.history.iter().enumerate() {
            println!("  {} {}", format!("[{}]", i + 1).dimmed(), cmd);
        }
    }
}
