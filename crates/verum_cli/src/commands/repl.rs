//! REPL command - Interactive Read-Eval-Print Loop for Verum
//!
//! # VBC-First Architecture Migration
//!
//! This REPL is being migrated to the VBC-first architecture. The old AST-walking
//! interpreter (`verum_interpreter`) has been deprecated in favor of `verum_vbc::interpreter`.
//!
//! Current status:
//! - Parsing: Works (using verum_parser)
//! - Type checking: Works (using verum_types)
//! - Evaluation: Pending VBC codegen integration
//!
//! For full execution, use `verum run <file.vr>` which will use the AOT pipeline.

use colored::Colorize;
use std::io::{self, Write};
use verum_common::{List, Map, Text};

use crate::error::Result;
use crate::ui;

use verum_ast::FileId;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// REPL state tracking (VBC-first version)
struct ReplState {
    parser: VerumParser,
    history: List<Text>,
    bindings: Map<Text, Text>,
    line_number: usize,
    multiline_buffer: Text,
    in_multiline: bool,
}

impl ReplState {
    fn new() -> Result<Self> {
        Ok(Self {
            parser: VerumParser::new(),
            history: List::new(),
            bindings: Map::new(),
            line_number: 1,
            multiline_buffer: Text::new(),
            in_multiline: false,
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
        // Print prompt
        print_prompt(&state);

        // Read input
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            println!();
            break;
        }

        let input = input.trim();

        // Check for REPL commands
        if input.starts_with(':') {
            if !handle_command(input, &mut state)? {
                break;
            }
            continue;
        }

        // Handle multiline input
        if input.ends_with('{') || state.in_multiline {
            state.multiline_buffer.push_str(input);
            state.multiline_buffer.push('\n');

            // Check if we should continue multiline
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
        "{}",
        "Note: REPL is in parse-only mode during VBC migration.".yellow()
    );
    println!(
        "{}",
        "For execution, use: verum run <file.vr>".yellow()
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
            // Clear screen
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
        ":ast" => {
            if parts.len() < 2 {
                ui::error("Usage: :ast <expression>");
            } else {
                let expr: String = parts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
                show_ast(&expr, state);
            }
        }
        ":status" => {
            print_vbc_status();
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
    println!("  {}       Reset environment", ":reset".cyan());
    println!("  {}      Show VBC migration status", ":status".cyan());
    println!();
    println!("{}", "VBC Migration Status:".bold().yellow());
    println!("  The REPL is currently in parse-only mode.");
    println!("  Expressions are parsed and validated but not executed.");
    println!("  For full execution, use: verum run <file.vr>");
    println!();
    println!("{}", "Examples (parse validation):".bold());
    println!("  let x = 42");
    println!("  x + 10");
    println!("  fn add(a: Int, b: Int) -> Int {{ a + b }}");
    println!("  :ast let x = 42");
    println!();
}

fn process_input(input: &str, state: &mut ReplState) {
    state.add_to_history(input);

    let file_id = FileId::new(state.line_number as u32);

    // Try to parse as expression first
    if let Ok(_expr) = state.parser.parse_expr_str(input, file_id) {
        println!(
            "{} {} {}",
            format!("[{}]", state.line_number).dimmed(),
            "✓".green(),
            "Parsed as expression".dimmed()
        );
        return;
    }

    // Try to parse as item
    let lexer = Lexer::new(input, file_id);
    let tokens: List<verum_lexer::Token> = lexer.filter_map(|r| r.ok()).collect();
    let tokens_list: List<_> = tokens.iter().cloned().collect();

    if let Ok(item) = state.parser.parse_item_tokens(&tokens_list) {
        match &item.kind {
            verum_ast::ItemKind::Function(func) => {
                let name = Text::from(func.name.name.as_str());
                state.bindings.insert(name.clone(), Text::from("Function"));
                println!(
                    "{} {} {}",
                    format!("[{}]", state.line_number).dimmed(),
                    "✓".green(),
                    format!("Defined function: {}", name.as_str().cyan())
                );
            }
            verum_ast::ItemKind::Type(type_decl) => {
                let name = Text::from(type_decl.name.name.as_str());
                state.bindings.insert(name.clone(), Text::from("Type"));
                println!(
                    "{} {} {}",
                    format!("[{}]", state.line_number).dimmed(),
                    "✓".green(),
                    format!("Defined type: {}", name.as_str().cyan())
                );
            }
            verum_ast::ItemKind::Protocol(proto) => {
                let name = Text::from(proto.name.name.as_str());
                state.bindings.insert(name.clone(), Text::from("Protocol"));
                println!(
                    "{} {} {}",
                    format!("[{}]", state.line_number).dimmed(),
                    "✓".green(),
                    format!("Defined protocol: {}", name.as_str().cyan())
                );
            }
            verum_ast::ItemKind::Impl(_) => {
                println!(
                    "{} {} {}",
                    format!("[{}]", state.line_number).dimmed(),
                    "✓".green(),
                    "Implementation defined".dimmed()
                );
            }
            _ => {
                println!(
                    "{} {} {}",
                    format!("[{}]", state.line_number).dimmed(),
                    "✓".green(),
                    "Parsed as item".dimmed()
                );
            }
        }
        return;
    }

    // If nothing worked, show parse error
    ui::error("Failed to parse input");
    ui::info("Try :help to see examples");
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

fn print_vbc_status() {
    println!();
    println!("{}", "VBC-First Architecture Migration Status".bold());
    println!("{}", "═".repeat(45).cyan());
    println!();
    println!("{} Parsing (verum_parser)", "✓".green());
    println!("{} Type checking (verum_types)", "✓".green());
    println!("{} VBC codegen (verum_vbc::codegen)", "◐".yellow());
    println!("{} VBC interpreter (verum_vbc::interpreter)", "✓".green());
    println!("{} REPL integration", "◐".yellow());
    println!();
    println!("{}", "Pipeline:".bold());
    println!("  Source → AST → {} → VBC → VBC Interpreter", "TypeCheck".cyan());
    println!();
    println!("{}", "For full execution:".bold());
    println!("  verum run <file.vr>     - Run via AOT pipeline");
    println!("  verum build <file.vr>   - Build executable");
    println!();
}

fn print_bindings(state: &ReplState) {
    if state.bindings.is_empty() {
        ui::info("No bindings defined");
    } else {
        println!("{}", "Current bindings:".bold());
        for (name, type_str) in &state.bindings {
            println!("  {} : {}", name.as_str().cyan(), type_str);
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
