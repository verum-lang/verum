//! Script context for tracking session state
//!
//! Moved from verum_parser::script

use verum_common::{List, Map, Text};

/// Context tracking for script parsing sessions
///
/// Maintains state across multiple REPL lines including:
/// - Defined bindings for tab completion
/// - Multiline input buffer
/// - Brace/bracket depth tracking
/// - Import statements
#[derive(Debug, Clone)]
pub struct ScriptContext {
    /// Accumulated multiline input
    pub buffer: Text,
    /// Defined variable names for completion (name -> type hint)
    pub bindings: Map<Text, Text>,
    /// Import statements from this session
    pub imports: List<Text>,
    /// Current brace depth (for completion detection)
    brace_depth: i32,
    /// Current bracket depth
    bracket_depth: i32,
    /// Current paren depth
    paren_depth: i32,
    /// Whether we're inside a string
    in_string: bool,
    /// Whether we're inside a multiline comment
    in_comment: bool,
    /// Line number in session
    pub line_number: usize,
    /// Definition tracking: maps binding names to the line numbers that define them
    /// This enables proper dependency detection and smart cache invalidation
    pub definition_lines: Map<Text, usize>,
    /// Function definitions with their line numbers
    pub function_lines: Map<Text, usize>,
    /// Type definitions with their line numbers
    pub type_lines: Map<Text, usize>,
}

impl ScriptContext {
    /// Create a new script context
    pub fn new() -> Self {
        Self {
            buffer: Text::new(),
            bindings: Map::new(),
            imports: List::new(),
            brace_depth: 0,
            bracket_depth: 0,
            paren_depth: 0,
            in_string: false,
            in_comment: false,
            line_number: 1,
            definition_lines: Map::new(),
            function_lines: Map::new(),
            type_lines: Map::new(),
        }
    }

    /// Register a variable definition at a specific line
    pub fn register_definition(&mut self, name: Text, line_number: usize, type_hint: Text) {
        self.bindings.insert(name.clone(), type_hint);
        self.definition_lines.insert(name, line_number);
    }

    /// Register a function definition at a specific line
    pub fn register_function(&mut self, name: Text, line_number: usize) {
        self.function_lines.insert(name, line_number);
    }

    /// Register a type definition at a specific line
    pub fn register_type(&mut self, name: Text, line_number: usize) {
        self.type_lines.insert(name, line_number);
    }

    /// Get the line number where a binding was defined
    pub fn get_definition_line(&self, name: &Text) -> Option<usize> {
        self.definition_lines.get(name).copied()
    }

    /// Get the line number where a function was defined
    pub fn get_function_line(&self, name: &Text) -> Option<usize> {
        self.function_lines.get(name).copied()
    }

    /// Get the line number where a type was defined
    pub fn get_type_line(&self, name: &Text) -> Option<usize> {
        self.type_lines.get(name).copied()
    }

    /// Get all definitions that were made on or after a specific line
    /// Used for cache invalidation when earlier lines are modified
    pub fn get_definitions_from_line(&self, start_line: usize) -> List<Text> {
        self.definition_lines
            .iter()
            .filter(|&(_, &line)| line >= start_line)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Remove all definitions from a specific line onward
    /// Used when invalidating cache from a modified line
    pub fn remove_definitions_from_line(&mut self, start_line: usize) {
        let names_to_remove: Vec<Text> = self
            .definition_lines
            .iter()
            .filter(|&(_, &line)| line >= start_line)
            .map(|(name, _)| name.clone())
            .collect();

        for name in names_to_remove {
            self.definition_lines.remove(&name);
            self.bindings.remove(&name);
        }

        // Also remove functions and types from that line onward
        let funcs_to_remove: Vec<Text> = self
            .function_lines
            .iter()
            .filter(|&(_, &line)| line >= start_line)
            .map(|(name, _)| name.clone())
            .collect();

        for name in funcs_to_remove {
            self.function_lines.remove(&name);
        }

        let types_to_remove: Vec<Text> = self
            .type_lines
            .iter()
            .filter(|&(_, &line)| line >= start_line)
            .map(|(name, _)| name.clone())
            .collect();

        for name in types_to_remove {
            self.type_lines.remove(&name);
        }
    }

    /// Add a line to the multiline buffer
    pub fn add_line(&mut self, line: &str) {
        if !self.buffer.is_empty() {
            self.buffer.push('\n');
        }
        self.buffer.push_str(line);
        self.update_depth(line);
    }

    /// Clear the multiline buffer
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.brace_depth = 0;
        self.bracket_depth = 0;
        self.paren_depth = 0;
        self.in_string = false;
        self.in_comment = false;
    }

    /// Check if input is complete (all brackets/braces closed)
    pub fn is_complete(&self) -> bool {
        !self.in_string
            && !self.in_comment
            && self.brace_depth == 0
            && self.bracket_depth == 0
            && self.paren_depth == 0
    }

    /// Update depth tracking for a line
    fn update_depth(&mut self, line: &str) {
        let mut chars = line.chars().peekable();
        let mut in_line_string = false;
        let mut escape_next = false;

        while let Some(ch) = chars.next() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_line_string || self.in_string => {
                    escape_next = true;
                }
                '"' if !self.in_comment => {
                    if in_line_string {
                        in_line_string = false;
                    } else if !self.in_string {
                        in_line_string = true;
                    }
                }
                '/' if !in_line_string && !self.in_string => {
                    if let Some(&next_ch) = chars.peek()
                        && next_ch == '*'
                    {
                        self.in_comment = true;
                        chars.next();
                    }
                }
                '*' if self.in_comment => {
                    if let Some(&next_ch) = chars.peek()
                        && next_ch == '/'
                    {
                        self.in_comment = false;
                        chars.next();
                    }
                }
                '{' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.brace_depth += 1;
                }
                '}' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.brace_depth -= 1;
                }
                '[' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.bracket_depth += 1;
                }
                ']' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.bracket_depth -= 1;
                }
                '(' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.paren_depth += 1;
                }
                ')' if !in_line_string && !self.in_string && !self.in_comment => {
                    self.paren_depth -= 1;
                }
                _ => {}
            }
        }

        // If we're still in a string at end of line, it's a multiline string
        if in_line_string {
            self.in_string = true;
        }
    }

    /// Add a binding from a let statement or function definition
    pub fn add_binding(&mut self, name: Text, type_hint: Text) {
        self.bindings.insert(name, type_hint);
    }

    /// Add an import statement
    pub fn add_import(&mut self, import: Text) {
        self.imports.push(import);
    }

    /// Get all binding names for completion
    pub fn get_bindings(&self) -> List<Text> {
        self.bindings.keys().cloned().collect()
    }

    /// Increment line number
    pub fn next_line(&mut self) {
        self.line_number += 1;
    }

    /// Reset the context for a new session
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.bindings.clear();
        self.imports.clear();
        self.brace_depth = 0;
        self.bracket_depth = 0;
        self.paren_depth = 0;
        self.in_string = false;
        self.in_comment = false;
        self.line_number = 1;
        self.definition_lines.clear();
        self.function_lines.clear();
        self.type_lines.clear();
    }

    /// Get current brace depth
    pub fn get_brace_depth(&self) -> i32 {
        self.brace_depth
    }

    /// Get current bracket depth
    pub fn get_bracket_depth(&self) -> i32 {
        self.bracket_depth
    }

    /// Get current paren depth
    pub fn get_paren_depth(&self) -> i32 {
        self.paren_depth
    }

    /// Check if we're inside a string
    pub fn is_in_string(&self) -> bool {
        self.in_string
    }

    /// Check if we're inside a comment
    pub fn is_in_comment(&self) -> bool {
        self.in_comment
    }
}

impl Default for ScriptContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_context_depth_tracking() {
        let mut ctx = ScriptContext::new();

        ctx.add_line("fn main() {");
        assert!(!ctx.is_complete());
        assert_eq!(ctx.brace_depth, 1);

        ctx.add_line("    let x = 42;");
        assert!(!ctx.is_complete());

        ctx.add_line("}");
        assert!(ctx.is_complete());
        assert_eq!(ctx.brace_depth, 0);
    }

    #[test]
    fn test_script_context_nested_braces() {
        let mut ctx = ScriptContext::new();

        ctx.add_line("let map = {");
        assert!(!ctx.is_complete());

        ctx.add_line("    'key': {");
        assert!(!ctx.is_complete());
        assert_eq!(ctx.brace_depth, 2);

        ctx.add_line("        'nested': 42");
        ctx.add_line("    }");
        assert!(!ctx.is_complete());
        assert_eq!(ctx.brace_depth, 1);

        ctx.add_line("};");
        assert!(ctx.is_complete());
    }

    #[test]
    fn test_context_string_tracking() {
        let mut ctx = ScriptContext::new();

        ctx.add_line(r#"let s = "hello {"#);
        // Should not count braces inside strings
        assert_eq!(ctx.brace_depth, 0);
        assert!(!ctx.is_complete()); // String not closed
    }

    #[test]
    fn test_context_comment_tracking() {
        let mut ctx = ScriptContext::new();

        ctx.add_line("/* { */ let x = 42");
        // Should not count braces in comments
        assert_eq!(ctx.brace_depth, 0);
        assert!(ctx.is_complete());
    }
}
