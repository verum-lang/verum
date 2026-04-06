//! Script mode parser for REPL and interactive sessions
//!
//! This module provides specialized parsing for script-like environments where:
//! - Expressions can be evaluated standalone
//! - Incremental parsing is essential for performance
//! - Partial input needs graceful handling
//! - Type inference should provide immediate feedback
//!
//! # Architecture
//!
//! The script parser wraps the main parser with additional features:
//! - **Expression-first parsing**: Try expressions before statements
//! - **Completion detection**: Identify incomplete vs. complete input
//! - **Context preservation**: Maintain state across REPL sessions
//! - **Smart recovery**: Handle common REPL errors gracefully
//!
//! # Example
//!
//! ```rust
//! use verum_lsp::script::{ScriptParser, ScriptContext, ParseMode};
//! use verum_ast::FileId;
//!
//! let mut parser = ScriptParser::new();
//! let mut context = ScriptContext::new();
//! let file_id = FileId::new(1);
//!
//! // Try parsing an expression
//! match parser.parse_line("let x = 42", file_id, &mut context) {
//!     Ok(result) => println!("Parsed: {:?}", result),
//!     Err(e) => eprintln!("Error: {:?}", e),
//! }
//!
//! // Check if input is complete
//! if !parser.is_complete("fn add(a: Int, b: Int) {") {
//!     println!("Waiting for more input...");
//! }
//! ```

use verum_ast::{FileId, Span};
use verum_common::{List, Maybe, Text};
use verum_lexer::{Lexer, Token};

use verum_parser::VerumParser;
use verum_parser::ParseError;
use verum_parser::RecursiveParser;

use super::context::ScriptContext;
use super::result::{ParseMode, ScriptParseResult};

/// Specialized parser for script/REPL environments
pub struct ScriptParser {
    /// Underlying Verum parser
    parser: VerumParser,
}

impl ScriptParser {
    /// Create a new script parser
    pub fn new() -> Self {
        Self {
            parser: VerumParser::new(),
        }
    }

    /// Parse a single line of script input
    ///
    /// This method tries to parse the input in the most appropriate way:
    /// 1. If the input is incomplete (open braces/brackets), return Incomplete
    /// 2. Try parsing as an expression (most common in REPL)
    /// 3. Try parsing as a statement (let bindings, etc.)
    /// 4. Try parsing as an item (functions, types)
    /// 5. Try parsing as a module (multiple items)
    pub fn parse_line(
        &self,
        input: &str,
        file_id: FileId,
        context: &mut ScriptContext,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        // Check if input is empty
        if input.trim().is_empty() {
            return Ok(ScriptParseResult::Empty);
        }

        // Add to context buffer
        context.add_line(input);

        // Check if complete
        if !context.is_complete() {
            return Ok(ScriptParseResult::Incomplete(context.buffer.clone()));
        }

        // Clone buffer to avoid borrow conflict
        let complete_input = context.buffer.clone();

        // Try parsing with auto mode
        self.parse_with_mode(&complete_input, file_id, ParseMode::Auto, context)
    }

    /// Parse input with a specific mode
    pub fn parse_with_mode(
        &self,
        input: &str,
        file_id: FileId,
        mode: ParseMode,
        context: &mut ScriptContext,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        match mode {
            ParseMode::Auto => self.parse_auto(input, file_id, context),
            ParseMode::Expression => self
                .parser
                .parse_expr_str(input, file_id)
                .map(ScriptParseResult::Expression),
            ParseMode::Statement => self.parse_statement(input, file_id, context),
            ParseMode::Item => self.parse_item(input, file_id, context),
            ParseMode::Module => {
                let lexer = Lexer::new(input, file_id);
                self.parser
                    .parse_module(lexer, file_id)
                    .map(ScriptParseResult::Module)
            }
        }
    }

    /// Try parsing in auto mode.
    ///
    /// Order: module (for multi-statement) → expression → statement → item.
    /// Module mode is tried FIRST when input contains `;` (a strong signal for
    /// multi-statement blocks like `println("ok"); let a = 1; println(a);`).
    fn parse_auto(
        &self,
        input: &str,
        file_id: FileId,
        context: &mut ScriptContext,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        let trimmed = input.trim();

        // If input looks like multiple statements (contains `;` not inside a string),
        // try module mode first to capture ALL statements.
        let has_multiple_semis = trimmed.matches(';').count() > 1
            || (trimmed.contains(';') && !trimmed.ends_with(';'));
        let has_semi_separated = trimmed.contains(';') && trimmed.len() > trimmed.find(';').unwrap_or(0) + 1;

        if has_semi_separated || has_multiple_semis {
            // Wrap in fn main() {} for module parsing if it's bare statements
            let needs_wrap = !trimmed.starts_with("fn ")
                && !trimmed.starts_with("type ")
                && !trimmed.starts_with("module ")
                && !trimmed.starts_with("mount ");

            if needs_wrap {
                let wrapped = format!("fn main() {{ {} }}", trimmed);
                let lexer = Lexer::new(&wrapped, file_id);
                if let Ok(module) = self.parser.parse_module(lexer, file_id) {
                    context.clear_buffer();
                    return Ok(ScriptParseResult::Module(module));
                }
            }

            // Try as-is (already a module with fn/type definitions)
            let lexer = Lexer::new(input, file_id);
            if let Ok(module) = self.parser.parse_module(lexer, file_id) {
                context.clear_buffer();
                return Ok(ScriptParseResult::Module(module));
            }
        }

        // 1. Try as expression
        if let Ok(expr) = self.parser.parse_expr_str(input, file_id) {
            context.clear_buffer();
            return Ok(ScriptParseResult::Expression(expr));
        }

        // 2. Try as statement
        if let Ok(result) = self.parse_statement(input, file_id, context) {
            return Ok(result);
        }

        // 3. Try as item
        if let Ok(result) = self.parse_item(input, file_id, context) {
            return Ok(result);
        }

        // 4. Try as module (fallback)
        let lexer = Lexer::new(input, file_id);
        match self.parser.parse_module(lexer, file_id) {
            Ok(module) => {
                context.clear_buffer();
                Ok(ScriptParseResult::Module(module))
            }
            Err(errors) => Err(errors),
        }
    }

    /// Parse as a statement
    fn parse_statement(
        &self,
        input: &str,
        file_id: FileId,
        context: &mut ScriptContext,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        let lexer = Lexer::new(input, file_id);
        let tokens: List<Token> = lexer.filter_map(|r| r.ok()).collect();

        let mut parser = RecursiveParser::new(&tokens, file_id);

        match parser.parse_stmt() {
            Ok(stmt) => {
                // Check if this is actually an item wrapped in a statement
                if let verum_ast::StmtKind::Item(item) = stmt.kind {
                    // Track item names for completion
                    match &item.kind {
                        verum_ast::ItemKind::Function(func) => {
                            let name = Text::from(func.name.name.as_str());
                            context.add_binding(name, Text::from("Function"));
                        }
                        verum_ast::ItemKind::Type(ty) => {
                            let name = Text::from(ty.name.name.as_str());
                            context.add_binding(name, Text::from("Type"));
                        }
                        verum_ast::ItemKind::Protocol(proto) => {
                            let name = Text::from(proto.name.name.as_str());
                            context.add_binding(name, Text::from("Protocol"));
                        }
                        _ => {}
                    }
                    context.clear_buffer();
                    return Ok(ScriptParseResult::Item(item));
                }

                // Extract bindings from let statements
                if let verum_ast::StmtKind::Let { pattern, ty, .. } = &stmt.kind
                    && let verum_ast::PatternKind::Ident {
                        name: ref ident, ..
                    } = pattern.kind
                {
                    let name = Text::from(ident.name.as_str());
                    let type_hint = if let Maybe::Some(type_ann) = ty {
                        Text::from(format!("{:?}", type_ann))
                    } else {
                        Text::from("<inferred>")
                    };
                    context.add_binding(name, type_hint);
                }

                context.clear_buffer();
                Ok(ScriptParseResult::Statement(stmt))
            }
            Err(e) => {
                if parser.errors.is_empty() {
                    Err(List::from(vec![e]))
                } else {
                    Err(parser.errors.into())
                }
            }
        }
    }

    /// Parse as an item (function, type, protocol, etc.)
    fn parse_item(
        &self,
        input: &str,
        file_id: FileId,
        context: &mut ScriptContext,
    ) -> Result<ScriptParseResult, List<ParseError>> {
        let lexer = Lexer::new(input, file_id);
        let tokens: List<Token> = lexer.filter_map(|r| r.ok()).collect();

        match self.parser.parse_item_tokens(&tokens) {
            Ok(item) => {
                // Track item names for completion
                match &item.kind {
                    verum_ast::ItemKind::Function(func) => {
                        let name = Text::from(func.name.name.as_str());
                        context.add_binding(name, Text::from("Function"));
                    }
                    verum_ast::ItemKind::Type(ty) => {
                        let name = Text::from(ty.name.name.as_str());
                        context.add_binding(name, Text::from("Type"));
                    }
                    verum_ast::ItemKind::Protocol(proto) => {
                        let name = Text::from(proto.name.name.as_str());
                        context.add_binding(name, Text::from("Protocol"));
                    }
                    _ => {}
                }

                context.clear_buffer();
                Ok(ScriptParseResult::Item(item))
            }
            Err(e) => {
                // Create a span that covers the entire input for syntax errors
                // This provides better error reporting than a dummy span
                let input_len = input.len() as u32;
                let error_span = Span::new(0, input_len, file_id);
                Err(List::from(vec![ParseError::invalid_syntax(
                    e.as_str(),
                    error_span,
                )]))
            }
        }
    }

    /// Check if input is complete (standalone function for quick checks)
    pub fn is_complete(&self, input: &str) -> bool {
        let mut temp_context = ScriptContext::new();
        temp_context.add_line(input);
        temp_context.is_complete()
    }

    /// Parse and check if input needs more lines
    ///
    /// Returns (is_complete, error_message_if_any)
    pub fn check_completeness(&self, input: &str) -> (bool, Maybe<Text>) {
        if input.trim().is_empty() {
            return (true, Maybe::None);
        }

        let mut context = ScriptContext::new();
        context.add_line(input);

        if !context.is_complete() {
            return (false, Maybe::None);
        }

        // Try parsing to detect errors
        let file_id = FileId::new(u32::MAX);
        match self.parse_auto(input, file_id, &mut context) {
            Ok(_) => (true, Maybe::None),
            Err(errors) => {
                let error_msg = errors
                    .iter()
                    .map(|e| format!("{}", e))
                    .collect::<Vec<_>>()
                    .join("; ");
                (true, Maybe::Some(Text::from(error_msg)))
            }
        }
    }

    /// Get the underlying parser for advanced use cases
    pub fn inner(&self) -> &VerumParser {
        &self.parser
    }
}

impl Default for ScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to detect if a line ends with a continuation indicator
pub fn needs_continuation(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.ends_with('{')
        || trimmed.ends_with('[')
        || trimmed.ends_with('(')
        || trimmed.ends_with(',')
        || trimmed.ends_with('|')
        || trimmed.ends_with('\\')
        || trimmed.ends_with("=>")
        || trimmed.ends_with("->")
}

/// Helper function to suggest completions based on partial input
pub fn suggest_completion(partial: &str, context: &ScriptContext) -> List<Text> {
    let mut suggestions = List::new();

    // Add context bindings that start with partial
    for binding in context.bindings.keys() {
        if binding.as_str().starts_with(partial) {
            suggestions.push(binding.clone());
        }
    }

    // Add common keywords
    let keywords = [
        "let", "fn", "type", "protocol", "impl", "match", "if", "else", "for", "while", "loop",
        "return", "break", "continue", "async", "await", "using", "provide", "context",
    ];

    for keyword in &keywords {
        if keyword.starts_with(partial) {
            suggestions.push(Text::from(*keyword));
        }
    }

    suggestions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_parser_expression() {
        let parser = ScriptParser::new();
        let mut ctx = ScriptContext::new();
        let file_id = FileId::new(1);

        let result = parser.parse_line("42 + 10", file_id, &mut ctx);
        assert!(result.is_ok());
        if let Ok(ScriptParseResult::Expression(_)) = result {
            // Success
        } else {
            panic!("Expected expression result");
        }
    }

    #[test]
    fn test_script_parser_incomplete_input() {
        let parser = ScriptParser::new();
        let mut ctx = ScriptContext::new();
        let file_id = FileId::new(1);

        let result = parser.parse_line("fn test() {", file_id, &mut ctx);
        assert!(result.is_ok());
        if let Ok(ScriptParseResult::Incomplete(_)) = result {
            // Expected incomplete
        } else {
            panic!("Expected incomplete result");
        }
    }

    #[test]
    fn test_is_complete_helper() {
        let parser = ScriptParser::new();

        assert!(parser.is_complete("let x = 42"));
        assert!(!parser.is_complete("fn test() {"));
        assert!(!parser.is_complete("let arr = ["));
        assert!(parser.is_complete("fn test() { }"));
    }

    #[test]
    fn test_needs_continuation() {
        assert!(needs_continuation("fn test() {"));
        assert!(needs_continuation("let arr = ["));
        assert!(needs_continuation("value,"));
        assert!(!needs_continuation("let x = 42"));
        assert!(!needs_continuation("fn test() { }"));
    }

    #[test]
    fn test_suggest_completion() {
        let mut ctx = ScriptContext::new();
        ctx.add_binding(Text::from("value"), Text::from("Int"));
        ctx.add_binding(Text::from("variable"), Text::from("Text"));

        let suggestions = suggest_completion("val", &ctx);
        assert!(suggestions.contains(&Text::from("value")));

        let suggestions = suggest_completion("le", &ctx);
        assert!(suggestions.contains(&Text::from("let")));
    }

    #[test]
    fn test_multiline_function() {
        let parser = ScriptParser::new();
        let mut ctx = ScriptContext::new();
        let file_id = FileId::new(1);

        // First line
        let r1 = parser.parse_line("fn add(a: Int, b: Int) -> Int {", file_id, &mut ctx);
        assert!(matches!(r1, Ok(ScriptParseResult::Incomplete(_))));

        // Second line
        let r2 = parser.parse_line("    a + b", file_id, &mut ctx);
        assert!(matches!(r2, Ok(ScriptParseResult::Incomplete(_))));

        // Final line
        let r3 = parser.parse_line("}", file_id, &mut ctx);
        assert!(r3.is_ok());
        if let Ok(ScriptParseResult::Item(_)) = r3 {
            // Expected item
        } else {
            panic!("Expected item result for complete function");
        }
    }
}
