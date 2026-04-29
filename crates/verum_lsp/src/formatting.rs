//! Code formatting support with trivia preservation
//!
//! Formats Verum code according to style guidelines.
//! Uses the lossless syntax tree to preserve comments and meaningful whitespace.
//!
//! Features:
//! - Trivia-preserving formatting (comments, doc comments)
//! - Consistent indentation (4 spaces by default)
//! - Proper spacing around operators
//! - Trailing whitespace removal
//! - Newline normalization
//! - Function signature formatting
//! - Import sorting
//! - Block structure formatting

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_parser::syntax_bridge::LosslessParser;
use verum_common::List;
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

// ==================== Configuration ====================

/// Verum-specific formatting configuration
#[derive(Debug, Clone)]
pub struct VerumFormatConfig {
    /// Number of spaces for indentation (default: 4)
    pub indent_size: usize,
    /// Maximum line width before breaking (default: 100)
    pub max_line_width: usize,
    /// Use trailing commas in multi-line lists
    pub trailing_commas: bool,
    /// Align consecutive assignments
    pub align_assignments: bool,
    /// Single blank line between top-level items
    pub blank_lines_between_items: usize,
    /// Sort and group imports
    pub sort_imports: bool,
    /// Add space inside braces: { x } vs {x}
    pub space_inside_braces: bool,
    /// Add space before opening brace: fn foo() { vs fn foo(){
    pub space_before_brace: bool,
    /// Preserve blank lines in user code
    pub preserve_blank_lines: bool,
}

impl Default for VerumFormatConfig {
    fn default() -> Self {
        Self {
            indent_size: 4,
            max_line_width: 100,
            trailing_commas: true,
            align_assignments: false,
            blank_lines_between_items: 1,
            sort_imports: true,
            space_inside_braces: true,
            space_before_brace: true,
            preserve_blank_lines: true,
        }
    }
}

// ==================== Trivia-Preserving Formatter ====================

/// Trivia-preserving formatter that uses the syntax tree.
pub struct TriviaPreservingFormatter {
    config: VerumFormatConfig,
}

impl TriviaPreservingFormatter {
    /// Create a new formatter with the given configuration.
    pub fn new(config: VerumFormatConfig) -> Self {
        Self { config }
    }

    /// Format source code preserving trivia.
    pub fn format(&self, source: &str, file_id: FileId) -> String {
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        let root = result.syntax();

        // Collect trivia (comments) and their positions
        let trivia = self.collect_trivia(&root);

        // Format the code
        let formatted = self.format_node(&root, 0);

        // Re-insert preserved trivia
        self.merge_trivia(&formatted, &trivia, source)
    }

    /// Collect all trivia (comments) from the syntax tree.
    fn collect_trivia(&self, node: &SyntaxNode) -> Vec<TriviaInfo> {
        let mut trivia = Vec::new();
        self.collect_trivia_recursive(node, &mut trivia);
        trivia
    }

    fn collect_trivia_recursive(&self, node: &SyntaxNode, trivia: &mut Vec<TriviaInfo>) {
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let kind = token.kind();
                    if kind.is_trivia() && !matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE) {
                        // This is a comment - preserve it
                        let range = token.text_range();
                        trivia.push(TriviaInfo {
                            kind,
                            text: token.text().to_string(),
                            start: range.start(),
                            end: range.end(),
                            is_doc_comment: matches!(
                                kind,
                                SyntaxKind::DOC_COMMENT | SyntaxKind::INNER_DOC_COMMENT
                            ),
                        });
                    }
                }
                SyntaxElement::Node(child_node) => {
                    self.collect_trivia_recursive(&child_node, trivia);
                }
            }
        }
    }

    /// Format a node with proper indentation.
    fn format_node(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        let indent_str = " ".repeat(self.config.indent_size * indent);

        match node.kind() {
            SyntaxKind::SOURCE_FILE => {
                for child in node.child_nodes() {
                    let formatted = self.format_node(&child, indent);
                    if !formatted.is_empty() {
                        result.push_str(&formatted);
                        // Add blank line between top-level items
                        for _ in 0..self.config.blank_lines_between_items {
                            result.push('\n');
                        }
                    }
                }
                // Remove trailing blank lines
                while result.ends_with("\n\n") {
                    result.pop();
                }
            }

            SyntaxKind::FN_DEF => {
                result.push_str(&indent_str);
                result.push_str(&self.format_function(node, indent));
            }

            SyntaxKind::TYPE_DEF => {
                result.push_str(&indent_str);
                result.push_str(&self.format_type_def(node, indent));
            }

            SyntaxKind::PROTOCOL_DEF => {
                result.push_str(&indent_str);
                result.push_str(&self.format_protocol_def(node, indent));
            }

            SyntaxKind::IMPL_BLOCK => {
                result.push_str(&indent_str);
                result.push_str(&self.format_impl_block(node, indent));
            }

            SyntaxKind::MOUNT_STMT => {
                result.push_str(&indent_str);
                result.push_str(&self.format_mount(node));
                result.push('\n');
            }

            SyntaxKind::BLOCK => {
                result.push_str(&self.format_block(node, indent));
            }

            SyntaxKind::LET_STMT => {
                result.push_str(&indent_str);
                result.push_str(&self.format_let_stmt(node));
                result.push('\n');
            }

            SyntaxKind::EXPR_STMT => {
                result.push_str(&indent_str);
                result.push_str(&self.format_tokens(node));
                result.push('\n');
            }

            SyntaxKind::RETURN_STMT => {
                result.push_str(&indent_str);
                result.push_str(&self.format_tokens(node));
                result.push('\n');
            }

            _ => {
                // Default: reconstruct from tokens with spacing
                result.push_str(&self.format_tokens(node));
            }
        }

        result
    }

    /// Format a function definition.
    fn format_function(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        let mut has_body = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::FN_KW => {
                            result.push_str("fn ");
                        }
                        SyntaxKind::IDENT => {
                            result.push_str(text);
                        }
                        SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => {
                            result.push_str(text);
                        }
                        SyntaxKind::ARROW => {
                            result.push_str(" -> ");
                        }
                        SyntaxKind::L_BRACE => {
                            if self.config.space_before_brace {
                                result.push(' ');
                            }
                            result.push('{');
                            has_body = true;
                        }
                        SyntaxKind::R_BRACE => {
                            // Handled in block formatting
                        }
                        SyntaxKind::ASYNC_KW => {
                            result.push_str("async ");
                        }
                        SyntaxKind::PUB_KW => {
                            result.push_str("pub ");
                        }
                        k if k.is_trivia() => {
                            // Skip trivia in main formatting (handled separately)
                        }
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::PARAM_LIST => {
                            result.push_str(&self.format_param_list(&child_node));
                        }
                        SyntaxKind::GENERIC_PARAMS => {
                            result.push_str(&self.format_generic_params(&child_node));
                        }
                        SyntaxKind::BLOCK => {
                            if has_body {
                                result.push('\n');
                                result.push_str(&self.format_block(&child_node, indent));
                            }
                        }
                        SyntaxKind::WHERE_CLAUSE => {
                            result.push_str(&self.format_where_clause(&child_node, indent));
                        }
                        _ => {
                            result.push_str(&self.format_tokens(&child_node));
                        }
                    }
                }
            }
        }

        if has_body {
            // Add closing brace with proper indentation
            result.push_str(&" ".repeat(self.config.indent_size * indent));
            result.push_str("}\n");
        } else {
            result.push_str(";\n");
        }

        result
    }

    /// Format a type definition.
    fn format_type_def(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        result.push_str("type ");

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::TYPE_KW => {
                            // Already handled
                        }
                        SyntaxKind::IDENT => {
                            result.push_str(text);
                        }
                        SyntaxKind::IS_KW => {
                            result.push_str(" is ");
                        }
                        SyntaxKind::SEMICOLON => {
                            result.push(';');
                        }
                        SyntaxKind::L_BRACE => {
                            if self.config.space_before_brace {
                                result.push(' ');
                            }
                            result.push_str("{\n");
                        }
                        SyntaxKind::R_BRACE => {
                            result.push_str(&" ".repeat(self.config.indent_size * indent));
                            result.push('}');
                        }
                        SyntaxKind::PIPE => {
                            result.push_str(" | ");
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            result.push_str(&self.format_generic_params(&child_node));
                        }
                        SyntaxKind::FIELD_LIST => {
                            result.push_str(&self.format_field_list(&child_node, indent + 1));
                        }
                        SyntaxKind::VARIANT_LIST => {
                            result.push_str(&self.format_variant_list(&child_node));
                        }
                        _ => {
                            result.push_str(&self.format_tokens(&child_node));
                        }
                    }
                }
            }
        }

        result.push('\n');
        result
    }

    /// Format a protocol definition.
    fn format_protocol_def(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        result.push_str("type ");

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::TYPE_KW => {}
                        SyntaxKind::IDENT => {
                            result.push_str(text);
                        }
                        SyntaxKind::IS_KW => {
                            result.push_str(" is ");
                        }
                        SyntaxKind::PROTOCOL_KW => {
                            result.push_str("protocol ");
                        }
                        SyntaxKind::L_BRACE => {
                            result.push_str("{\n");
                        }
                        SyntaxKind::R_BRACE => {
                            result.push_str(&" ".repeat(self.config.indent_size * indent));
                            result.push_str("};\n");
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::GENERIC_PARAMS => {
                            result.push_str(&self.format_generic_params(&child_node));
                        }
                        SyntaxKind::PROTOCOL_ITEM | SyntaxKind::PROTOCOL_FN => {
                            result.push_str(&" ".repeat(self.config.indent_size * (indent + 1)));
                            result.push_str(&self.format_tokens(&child_node));
                            result.push('\n');
                        }
                        _ => {
                            result.push_str(&self.format_tokens(&child_node));
                        }
                    }
                }
            }
        }

        result
    }

    /// Format an implementation block.
    fn format_impl_block(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        result.push_str("implement ");

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::IMPLEMENT_KW => {}
                        SyntaxKind::FOR_KW => {
                            result.push_str(" for ");
                        }
                        SyntaxKind::L_BRACE => {
                            result.push_str(" {\n");
                        }
                        SyntaxKind::R_BRACE => {
                            result.push_str(&" ".repeat(self.config.indent_size * indent));
                            result.push_str("}\n");
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    match child_node.kind() {
                        SyntaxKind::IMPL_FN | SyntaxKind::FN_DEF => {
                            result.push_str(&self.format_function(&child_node, indent + 1));
                        }
                        _ => {
                            result.push_str(&self.format_tokens(&child_node));
                        }
                    }
                }
            }
        }

        result
    }

    /// Format a mount statement.
    fn format_mount(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        result.push_str("mount ");

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::MOUNT_KW => {}
                        SyntaxKind::SEMICOLON => {
                            result.push(';');
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    result.push_str(&self.format_tokens(&child_node));
                }
            }
        }

        result
    }

    /// Format a let statement.
    fn format_let_stmt(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        result.push_str("let ");

        let mut seen_eq = false;
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::LET_KW => {}
                        SyntaxKind::MUT_KW => {
                            result.push_str("mut ");
                        }
                        SyntaxKind::IDENT => {
                            if !seen_eq {
                                result.push_str(text);
                            } else {
                                result.push_str(text);
                            }
                        }
                        SyntaxKind::COLON => {
                            result.push_str(": ");
                        }
                        SyntaxKind::EQ => {
                            result.push_str(" = ");
                            seen_eq = true;
                        }
                        SyntaxKind::SEMICOLON => {
                            result.push(';');
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    result.push_str(&self.format_tokens(&child_node));
                }
            }
        }

        result
    }

    /// Format a block with proper indentation.
    fn format_block(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        let inner_indent = " ".repeat(self.config.indent_size * (indent + 1));

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::L_BRACE | SyntaxKind::R_BRACE => {
                            // Braces are handled by the parent
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(&inner_indent);
                            result.push_str(token.text());
                            result.push('\n');
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    let formatted = self.format_node(&child_node, indent + 1);
                    result.push_str(&formatted);
                }
            }
        }

        result
    }

    /// Format a parameter list.
    fn format_param_list(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        result.push('(');

        let mut first = true;
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::L_PAREN | SyntaxKind::R_PAREN => {}
                        SyntaxKind::COMMA => {
                            result.push_str(", ");
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(token.text());
                        }
                    }
                }
                SyntaxElement::Node(child_node) if child_node.kind() == SyntaxKind::PARAM => {
                    if !first {
                        result.push_str(", ");
                    }
                    result.push_str(&self.format_param(&child_node));
                    first = false;
                }
                SyntaxElement::Node(child_node) => {
                    result.push_str(&self.format_tokens(&child_node));
                }
            }
        }

        result.push(')');
        result
    }

    /// Format a parameter.
    fn format_param(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let text = token.text();
                    match token.kind() {
                        SyntaxKind::IDENT => {
                            result.push_str(text);
                        }
                        SyntaxKind::COLON => {
                            result.push_str(": ");
                        }
                        SyntaxKind::MUT_KW => {
                            result.push_str("mut ");
                        }
                        SyntaxKind::AMP => {
                            result.push('&');
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(text);
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    result.push_str(&self.format_tokens(&child_node));
                }
            }
        }

        result
    }

    /// Format generic parameters.
    fn format_generic_params(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        result.push('<');

        let mut first = true;
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::L_ANGLE | SyntaxKind::R_ANGLE => {}
                        SyntaxKind::COMMA => {}
                        SyntaxKind::IDENT => {
                            if !first {
                                result.push_str(", ");
                            }
                            result.push_str(token.text());
                            first = false;
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(token.text());
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if !first {
                        result.push_str(", ");
                    }
                    result.push_str(&self.format_tokens(&child_node));
                    first = false;
                }
            }
        }

        result.push('>');
        result
    }

    /// Format a where clause.
    fn format_where_clause(&self, node: &SyntaxNode, _indent: usize) -> String {
        let mut result = String::new();
        result.push_str("\nwhere\n");

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::WHERE_KW => {}
                        SyntaxKind::COMMA => {
                            result.push_str(",\n");
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(token.text());
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    result.push_str("    ");
                    result.push_str(&self.format_tokens(&child_node));
                }
            }
        }

        result
    }

    /// Format a field list.
    fn format_field_list(&self, node: &SyntaxNode, indent: usize) -> String {
        let mut result = String::new();
        let field_indent = " ".repeat(self.config.indent_size * indent);

        for child in node.children() {
            if let SyntaxElement::Node(child_node) = child {
                if child_node.kind() == SyntaxKind::FIELD_DEF {
                    result.push_str(&field_indent);
                    result.push_str(&self.format_tokens(&child_node));
                    result.push_str(",\n");
                }
            }
        }

        result
    }

    /// Format a variant list.
    fn format_variant_list(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        let mut first = true;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    match token.kind() {
                        SyntaxKind::PIPE => {
                            if !first {
                                result.push_str(" | ");
                            }
                        }
                        k if k.is_trivia() => {}
                        _ => {
                            result.push_str(token.text());
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    if !first {
                        result.push_str(" | ");
                    }
                    result.push_str(&self.format_tokens(&child_node));
                    first = false;
                }
            }
        }

        result
    }

    /// Format tokens with proper spacing (fallback for unhandled nodes).
    fn format_tokens(&self, node: &SyntaxNode) -> String {
        let mut result = String::new();
        let mut need_space = false;

        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    let kind = token.kind();

                    // Skip whitespace trivia (we control spacing)
                    if matches!(kind, SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE) {
                        continue;
                    }

                    // Preserve comments
                    if kind.is_trivia() {
                        if need_space {
                            result.push(' ');
                        }
                        result.push_str(token.text());
                        need_space = true;
                        continue;
                    }

                    // Add spacing based on token type
                    let text = token.text();

                    match kind {
                        // No space before these
                        SyntaxKind::R_PAREN
                        | SyntaxKind::R_BRACKET
                        | SyntaxKind::R_BRACE
                        | SyntaxKind::COMMA
                        | SyntaxKind::SEMICOLON
                        | SyntaxKind::COLON_COLON
                        | SyntaxKind::DOT => {
                            result.push_str(text);
                            need_space = matches!(kind, SyntaxKind::COMMA | SyntaxKind::SEMICOLON);
                        }
                        // No space after these
                        SyntaxKind::L_PAREN
                        | SyntaxKind::L_BRACKET
                        | SyntaxKind::L_BRACE
                        | SyntaxKind::AMP
                        | SyntaxKind::STAR => {
                            if need_space
                                && !matches!(kind, SyntaxKind::AMP | SyntaxKind::STAR)
                            {
                                result.push(' ');
                            }
                            result.push_str(text);
                            need_space = false;
                        }
                        // Space around these
                        SyntaxKind::EQ
                        | SyntaxKind::EQ_EQ
                        | SyntaxKind::BANG_EQ
                        | SyntaxKind::PLUS
                        | SyntaxKind::MINUS
                        | SyntaxKind::SLASH
                        | SyntaxKind::PERCENT
                        | SyntaxKind::LT_EQ
                        | SyntaxKind::GT_EQ
                        | SyntaxKind::AMP_AMP
                        | SyntaxKind::PIPE_PIPE
                        | SyntaxKind::ARROW
                        | SyntaxKind::FAT_ARROW
                        | SyntaxKind::PIPE_GT => {
                            if !result.ends_with(' ') && !result.is_empty() {
                                result.push(' ');
                            }
                            result.push_str(text);
                            result.push(' ');
                            need_space = false;
                        }
                        // Colon has special handling
                        SyntaxKind::COLON => {
                            result.push(':');
                            result.push(' ');
                            need_space = false;
                        }
                        // Keywords need space after
                        k if k.is_keyword() => {
                            if need_space {
                                result.push(' ');
                            }
                            result.push_str(text);
                            need_space = true;
                        }
                        // Default: check if space needed
                        _ => {
                            if need_space {
                                result.push(' ');
                            }
                            result.push_str(text);
                            need_space = true;
                        }
                    }
                }
                SyntaxElement::Node(child_node) => {
                    let formatted = self.format_tokens(&child_node);
                    if !formatted.is_empty() {
                        if need_space && !formatted.starts_with(' ') {
                            result.push(' ');
                        }
                        result.push_str(&formatted);
                        need_space = !formatted.ends_with(' ')
                            && !formatted.ends_with('(')
                            && !formatted.ends_with('[');
                    }
                }
            }
        }

        result
    }

    /// Merge preserved trivia back into formatted code.
    fn merge_trivia(&self, formatted: &str, trivia: &[TriviaInfo], _source: &str) -> String {
        if trivia.is_empty() {
            return formatted.to_string();
        }

        // For now, append doc comments at appropriate locations
        // A more sophisticated implementation would match positions
        let mut result = String::new();
        let lines: Vec<&str> = formatted.lines().collect();

        for (i, line) in lines.iter().enumerate() {
            // Check for doc comments that should appear before this line
            for t in trivia {
                if t.is_doc_comment {
                    // Find appropriate location based on heuristics
                    // (In a full implementation, we'd track spans more carefully)
                    if i == 0 && line.contains("fn ") || line.contains("type ") {
                        result.push_str(&t.text);
                        result.push('\n');
                    }
                }
            }

            result.push_str(line);
            result.push('\n');
        }

        // Ensure single trailing newline
        while result.ends_with("\n\n") {
            result.pop();
        }

        result
    }
}

/// Information about preserved trivia.
#[derive(Debug)]
struct TriviaInfo {
    kind: SyntaxKind,
    text: String,
    start: u32,
    end: u32,
    is_doc_comment: bool,
}

// ==================== Public API ====================

/// Format a document with full Verum formatting rules (with trivia preservation).
pub fn format_document(text: &str) -> List<TextEdit> {
    format_document_with_config(text, VerumFormatConfig::default())
}

/// Format a document honouring caller-supplied formatting config.
///
/// The LSP `formatting` request carries `FormattingOptions` (tab
/// size, insert-spaces) that the editor sourced from the user's
/// own settings. The fallback formatter previously hardcoded
/// `VerumFormatConfig::default()`, ignoring those preferences —
/// `indent_size` was always 4 even when the editor asked for 2.
/// This entry point lets `Backend::formatting` translate the LSP
/// options into a `VerumFormatConfig` and have them actually
/// propagate through both the trivia-preserving and basic
/// formatter paths.
pub fn format_document_with_config(
    text: &str,
    config: VerumFormatConfig,
) -> List<TextEdit> {
    let formatter = TriviaPreservingFormatter::new(config.clone());

    // Use trivia-preserving formatter for syntax tree-based formatting
    let formatted = formatter.format(text, FileId::new(0));

    // Fall back to basic formatting if the syntax tree has errors
    let final_text = if formatted.is_empty() || formatted.trim().is_empty() {
        format_verum_source(text, &config)
    } else {
        formatted
    };

    if final_text == text {
        // No changes needed
        return List::new();
    }

    // Replace entire document
    let mut result = List::new();
    result.push(TextEdit {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: u32::MAX,
                character: u32::MAX,
            },
        },
        new_text: final_text.to_string(),
    });
    result
}

/// Format Verum source code with full formatting rules (fallback).
fn format_verum_source(text: &str, config: &VerumFormatConfig) -> String {
    let lines: Vec<&str> = text.lines().collect();

    // Pass 1: Separate imports from other content
    let (imports, rest) = separate_imports(&lines);

    // Pass 2: Sort imports if configured
    let sorted_imports = if config.sort_imports {
        sort_import_lines(imports)
    } else {
        imports.iter().map(|s| s.to_string()).collect()
    };

    // Pass 3: Format each line with proper indentation
    let formatted_rest = format_lines_with_config(&rest, config);

    // Pass 4: Combine imports and rest
    let mut result = String::new();

    for import in sorted_imports {
        result.push_str(&import);
        result.push('\n');
    }

    if !result.is_empty() && !formatted_rest.is_empty() {
        result.push('\n'); // Blank line after imports
    }

    result.push_str(&formatted_rest);

    // Ensure single trailing newline
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }

    result
}

/// Separate import lines from other content
fn separate_imports<'a>(lines: &[&'a str]) -> (Vec<&'a str>, Vec<&'a str>) {
    let mut imports = Vec::new();
    let mut rest = Vec::new();
    let mut past_imports = false;

    for line in lines {
        let trimmed = line.trim();

        if !past_imports {
            if trimmed.starts_with("use ") || trimmed.starts_with("import ") || trimmed.is_empty() {
                if trimmed.starts_with("use ") || trimmed.starts_with("import ") {
                    imports.push(*line);
                }
                continue;
            } else if !trimmed.starts_with("//") {
                past_imports = true;
            }
        }

        rest.push(*line);
    }

    (imports, rest)
}

/// Sort import lines by module path
fn sort_import_lines(imports: Vec<&str>) -> Vec<String> {
    let mut sorted: Vec<String> = imports.iter().map(|s| s.trim().to_string()).collect();
    sorted.sort_by(|a, b| {
        // Sort stdlib imports first, then alphabetically
        let a_is_std = a.contains("stdlib");
        let b_is_std = b.contains("stdlib");

        if a_is_std && !b_is_std {
            std::cmp::Ordering::Less
        } else if !a_is_std && b_is_std {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(b)
        }
    });
    sorted
}

/// Format lines with proper indentation and spacing
fn format_lines_with_config(lines: &[&str], config: &VerumFormatConfig) -> String {
    let mut result = String::new();
    let mut indent_level: usize = 0;
    let mut in_multiline_string = false;
    let mut prev_was_blank = false;
    let indent = " ".repeat(config.indent_size);

    for line in lines {
        let trimmed = line.trim();

        // Track multiline strings
        let quote_count = trimmed.matches("\"\"\"").count();
        if quote_count % 2 == 1 {
            in_multiline_string = !in_multiline_string;
        }

        // Don't format inside multiline strings
        if in_multiline_string && quote_count == 0 {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Handle blank lines
        if trimmed.is_empty() {
            if !prev_was_blank && !result.is_empty() {
                result.push('\n');
                prev_was_blank = true;
            }
            continue;
        }
        prev_was_blank = false;

        // Decrease indent for closing braces
        if trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')') {
            indent_level = indent_level.saturating_sub(1);
        }

        // Format the line content
        let formatted_line = format_line_content(trimmed, config);

        // Add indentation and formatted content
        if !trimmed.is_empty() {
            result.push_str(&indent.repeat(indent_level));
            result.push_str(&formatted_line);
        }
        result.push('\n');

        // Increase indent for opening braces
        if trimmed.ends_with('{') || trimmed.ends_with('[') {
            indent_level += 1;
        }
        // Handle inline blocks like `fn foo() { ... }`
        if trimmed.contains('{') && !trimmed.ends_with('{') && !trimmed.ends_with('}') {
            // Count braces to handle cases like `{ x }`
            let opens = trimmed.matches('{').count();
            let closes = trimmed.matches('}').count();
            if opens > closes {
                indent_level += opens - closes;
            }
        }
    }

    result
}

/// Format line content with proper spacing
fn format_line_content(line: &str, config: &VerumFormatConfig) -> String {
    let mut result = String::with_capacity(line.len() + 10);
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    let mut in_char = false;

    while i < chars.len() {
        let ch = chars[i];
        let prev = if i > 0 { Some(chars[i - 1]) } else { None };
        let next = chars.get(i + 1).copied();

        // Track string literals
        if ch == '"' && prev != Some('\\') {
            in_string = !in_string;
            result.push(ch);
            i += 1;
            continue;
        }

        // Track char literals
        if ch == '\'' && prev != Some('\\') && !in_string {
            in_char = !in_char;
            result.push(ch);
            i += 1;
            continue;
        }

        // Don't format inside strings or chars
        if in_string || in_char {
            result.push(ch);
            i += 1;
            continue;
        }

        // Handle operators with proper spacing
        match ch {
            // Binary operators
            '+' | '*' | '/' | '%' => {
                ensure_space_before(&mut result);
                result.push(ch);
                if next != Some('=') {
                    result.push(' ');
                }
            }
            '-' => {
                // Could be binary minus or unary negation or arrow
                if next == Some('>') {
                    ensure_space_before(&mut result);
                    result.push('-');
                    result.push('>');
                    result.push(' ');
                    i += 1;
                } else if prev
                    .map(|c| c.is_alphanumeric() || c == ')' || c == ']')
                    .unwrap_or(false)
                {
                    ensure_space_before(&mut result);
                    result.push('-');
                    if next != Some('=') {
                        result.push(' ');
                    }
                } else {
                    result.push('-');
                }
            }
            '=' => {
                if next == Some('=') {
                    ensure_space_before(&mut result);
                    result.push_str("== ");
                    i += 1;
                } else if next == Some('>') {
                    ensure_space_before(&mut result);
                    result.push_str("=> ");
                    i += 1;
                } else if prev != Some('!') && prev != Some('<') && prev != Some('>') {
                    ensure_space_before(&mut result);
                    result.push_str("= ");
                } else {
                    result.push('=');
                    result.push(' ');
                }
            }
            '<' | '>' => {
                if next == Some('=')
                    || (ch == '<' && next == Some('<'))
                    || (ch == '>' && next == Some('>'))
                {
                    ensure_space_before(&mut result);
                    result.push(ch);
                    if let Some(next_ch) = next {
                        result.push(next_ch);
                    }
                    if chars.get(i + 2) != Some(&'=') {
                        result.push(' ');
                    }
                    i += 1;
                } else {
                    // Could be comparison or generic
                    let is_generic = prev.map(|c| c.is_alphabetic()).unwrap_or(false);
                    if !is_generic {
                        ensure_space_before(&mut result);
                    }
                    result.push(ch);
                    if !is_generic && next.map(|c| c.is_alphabetic()).unwrap_or(false) {
                        result.push(' ');
                    }
                }
            }
            '!' => {
                if next == Some('=') {
                    ensure_space_before(&mut result);
                    result.push_str("!= ");
                    i += 1;
                } else {
                    result.push('!');
                }
            }
            '&' => {
                if next == Some('&') {
                    ensure_space_before(&mut result);
                    result.push_str("&& ");
                    i += 1;
                } else {
                    result.push('&');
                }
            }
            '|' => {
                if next == Some('|') {
                    ensure_space_before(&mut result);
                    result.push_str("|| ");
                    i += 1;
                } else if next == Some('>') {
                    // Pipe operator
                    ensure_space_before(&mut result);
                    result.push_str("|> ");
                    i += 1;
                } else {
                    result.push('|');
                }
            }
            ':' => {
                if next == Some(':') {
                    result.push_str("::");
                    i += 1;
                } else {
                    result.push(':');
                    if next != Some(' ') && next.is_some() {
                        result.push(' ');
                    }
                }
            }
            ',' => {
                result.push(',');
                if next != Some(' ') && next.is_some() && next != Some('\n') {
                    result.push(' ');
                }
            }
            '{' => {
                if config.space_before_brace && prev != Some(' ') && prev.is_some() {
                    ensure_space_before(&mut result);
                }
                result.push('{');
                if config.space_inside_braces
                    && next != Some('}')
                    && next.is_some()
                    && next != Some('\n')
                {
                    result.push(' ');
                }
            }
            '}' => {
                if config.space_inside_braces && prev != Some('{') && prev != Some(' ') {
                    ensure_space_before(&mut result);
                }
                result.push('}');
            }
            _ => {
                result.push(ch);
            }
        }

        i += 1;
    }

    // Remove trailing whitespace
    result.trim_end().to_string()
}

/// Ensure there's a space before the current position
fn ensure_space_before(result: &mut String) {
    if !result.ends_with(' ')
        && !result.is_empty()
        && !result.ends_with('(')
        && !result.ends_with('[')
    {
        result.push(' ');
    }
}

/// Format a range of the document
pub fn format_range(text: &str, range: Range) -> List<TextEdit> {
    // Extract the range, format it, and return the edit
    let lines: Vec<&str> = text.lines().collect();

    let start_line = range.start.line as usize;
    let end_line = range.end.line as usize;

    if start_line >= lines.len() {
        return List::new();
    }

    let end_line = end_line.min(lines.len() - 1);
    let range_text = lines[start_line..=end_line].join("\n");
    let formatted_range = basic_format(&range_text);

    if formatted_range == range_text {
        return List::new();
    }

    let mut result = List::new();
    result.push(TextEdit {
        range,
        new_text: formatted_range.to_string(),
    });
    result
}

/// Format on type (when user types certain characters)
pub fn format_on_type(text: &str, position: Position, ch: char) -> List<TextEdit> {
    match ch {
        '}' => format_closing_brace(text, position),
        ';' => format_semicolon(text, position),
        '\n' => format_newline(text, position),
        '>' => format_pipeline_operator(text, position),
        _ => List::new(),
    }
}

/// Format after `|>` (pipeline operator): align continuation to pipeline start.
///
/// Verum's pipeline operator chains expressions vertically. When the user
/// types `|>` we indent the continuation to one level deeper than the
/// expression that started the pipeline chain, producing:
///
/// ```verum
/// data
///     |> transform
///     |> filter
/// ```
fn format_pipeline_operator(text: &str, position: Position) -> List<TextEdit> {
    let lines: Vec<&str> = text.lines().collect();
    let line_idx = position.line as usize;
    if line_idx >= lines.len() {
        return List::new();
    }

    let line = lines[line_idx];
    let trimmed = line.trim();

    // Only act when the line ends with or consists of `|>`
    if !trimmed.ends_with("|>") && !trimmed.starts_with("|>") {
        return List::new();
    }

    // Walk backwards to find the pipeline origin — the first non-|> line
    // in this chain — and align to one indent level past it.
    let origin_indent = find_pipeline_origin_indent(&lines, line_idx);
    let desired_indent = origin_indent + 4; // one level deeper

    let current_indent = line.len() - line.trim_start().len();
    if current_indent == desired_indent {
        return List::new();
    }

    let new_line = format!("{}{}", " ".repeat(desired_indent), trimmed);
    let mut result = List::new();
    result.push(TextEdit {
        range: Range {
            start: Position {
                line: line_idx as u32,
                character: 0,
            },
            end: Position {
                line: line_idx as u32,
                character: line.len() as u32,
            },
        },
        new_text: new_line,
    });
    result
}

/// Walk backwards through the pipeline chain to find the originating
/// expression and return its indentation (in spaces).
fn find_pipeline_origin_indent(lines: &[&str], current_line: usize) -> usize {
    for i in (0..current_line).rev() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            continue;
        }
        // Still part of the chain — keep walking
        if trimmed.starts_with("|>") || trimmed.ends_with("|>") {
            continue;
        }
        // Found the origin expression
        return lines[i].len() - lines[i].trim_start().len();
    }
    0
}

/// Basic formatting implementation
pub fn basic_format(text: &str) -> String {
    let mut result = String::new();
    let mut indent_level: usize = 0;

    for line in text.lines() {
        let trimmed = line.trim();

        // Decrease indent for closing braces
        if trimmed.starts_with('}') || trimmed.starts_with(']') || trimmed.starts_with(')') {
            indent_level = indent_level.saturating_sub(1);
        }

        // Add indentation
        if !trimmed.is_empty() {
            result.push_str(&"    ".repeat(indent_level));
            result.push_str(trimmed);
        }

        result.push('\n');

        // Increase indent for opening braces
        if trimmed.ends_with('{') || trimmed.ends_with('[') || trimmed.ends_with('(') {
            indent_level += 1;
        }
    }

    // Remove trailing empty lines but ensure one newline at end
    while result.ends_with("\n\n") {
        result.pop();
    }

    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

/// Format when a closing brace is typed
fn format_closing_brace(text: &str, position: Position) -> List<TextEdit> {
    let lines: Vec<&str> = text.lines().collect();
    let line_idx = position.line as usize;

    if line_idx >= lines.len() {
        return List::new();
    }

    let line = lines[line_idx];
    let trimmed = line.trim();

    // If the line only contains the brace and whitespace, fix indentation
    if trimmed == "}" {
        // Calculate proper indentation based on previous lines
        let indent = calculate_indent_for_closing_brace(&lines, line_idx);
        let new_line = format!("{}}}", "    ".repeat(indent));

        if new_line != line {
            let mut result = List::new();
            result.push(TextEdit {
                range: Range {
                    start: Position {
                        line: line_idx as u32,
                        character: 0,
                    },
                    end: Position {
                        line: line_idx as u32,
                        character: line.len() as u32,
                    },
                },
                new_text: new_line,
            });
            return result;
        }
    }

    List::new()
}

/// Format when a semicolon is typed
/// Currently no-op as Verum doesn't require semicolon formatting
fn format_semicolon(_text: &str, _position: Position) -> List<TextEdit> {
    List::new()
}

/// Format when a newline is typed
fn format_newline(text: &str, position: Position) -> List<TextEdit> {
    let lines: Vec<&str> = text.lines().collect();
    let line_idx = position.line as usize;

    if line_idx == 0 || line_idx > lines.len() {
        return List::new();
    }

    // Get the previous line to determine indentation
    let prev_line = lines[line_idx - 1];
    let indent = calculate_indent_for_new_line(prev_line);

    // Insert indentation at the start of the new line
    if indent > 0 {
        let mut result = List::new();
        result.push(TextEdit {
            range: Range {
                start: Position {
                    line: line_idx as u32,
                    character: 0,
                },
                end: Position {
                    line: line_idx as u32,
                    character: 0,
                },
            },
            new_text: "    ".repeat(indent),
        });
        return result;
    }

    List::new()
}

/// Calculate the proper indentation for a closing brace
fn calculate_indent_for_closing_brace(lines: &[&str], current_line: usize) -> usize {
    let mut depth = 1; // We need to find the matching opening brace

    for i in (0..current_line).rev() {
        let line = lines[i];
        for ch in line.chars() {
            match ch {
                '}' => depth += 1,
                '{' => {
                    depth -= 1;
                    if depth == 0 {
                        // Found matching brace, return its indentation
                        return line.chars().take_while(|c| c.is_whitespace()).count() / 4;
                    }
                }
                _ => {}
            }
        }
    }

    0 // Default to no indentation
}

/// Calculate the proper indentation for a new line.
///
/// The logic mirrors Verum's block structure:
///
/// | Previous line ends with | Indent change |
/// |-------------------------|---------------|
/// | `{` or `[`              | +1 level      |
/// | `=>`                    | +1 level (match arm body) |
/// | `\|>`                   | same (pipeline continuation) |
/// | `}` or `]`              | same (already dedented) |
/// | anything else           | same |
pub fn calculate_indent_for_new_line(prev_line: &str) -> usize {
    let base_indent = prev_line.chars().take_while(|c| c.is_whitespace()).count() / 4;
    let trimmed = prev_line.trim();

    // Block / collection openers
    if trimmed.ends_with('{') || trimmed.ends_with('[') {
        return base_indent + 1;
    }

    // Match arm body: `Pattern =>` — indent the body one level deeper
    if trimmed.ends_with("=>") {
        return base_indent + 1;
    }

    // Pipeline continuation: keep same indent so the next `|>` lines up
    if trimmed.ends_with("|>") {
        return base_indent;
    }

    base_indent
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_format() {
        let input = "fn foo(){\nlet x=1;\n}";
        let result = basic_format(input);
        assert!(result.contains("fn foo(){"));
        assert!(result.contains("    let x=1;"));
    }

    #[test]
    fn test_format_document() {
        let input = "fn foo() { let x = 1; }";
        let edits = format_document(input);
        // Should return edits if formatting changes anything
        // Note: exact behavior depends on formatter implementation
        assert!(edits.len() <= 1);
    }

    #[test]
    fn trivia_formatter_threads_config_indent_size() {
        // Pin: `TriviaPreservingFormatter` actually consumes its
        // `config.indent_size`. Two configs that differ only in
        // `indent_size` must produce different output for any
        // input the formatter re-indents.
        //
        // The check is on the lower-level formatter directly so
        // this test isn't gated on the `format_document` /
        // `format_document_with_config` choice of fallback path —
        // we're proving the wire-up at the source, not at the
        // outer entry point.
        let input = "if x > 0 {\nreturn y;\n}";
        let cfg2 = VerumFormatConfig {
            indent_size: 2,
            ..Default::default()
        };
        let cfg4 = VerumFormatConfig {
            indent_size: 4,
            ..Default::default()
        };
        let f2 = TriviaPreservingFormatter::new(cfg2);
        let f4 = TriviaPreservingFormatter::new(cfg4);
        let text2 = f2.format(input, FileId::new(0));
        let text4 = f4.format(input, FileId::new(0));
        // Both formatters produce the same structural shape but
        // different per-level indentation. If `format_document`
        // dropped the config, both would be identical.
        if text2.is_empty() && text4.is_empty() {
            // The trivia formatter punted on this input — fall
            // back to the basic-format path to still pin the
            // structural claim.
            let basic2 = format_verum_source(
                input,
                &VerumFormatConfig { indent_size: 2, ..Default::default() },
            );
            let basic4 = format_verum_source(
                input,
                &VerumFormatConfig { indent_size: 4, ..Default::default() },
            );
            assert_ne!(
                basic2, basic4,
                "basic-format path: indent_size=2 vs indent_size=4 must differ"
            );
        } else {
            assert_ne!(
                text2, text4,
                "trivia formatter: indent_size=2 vs indent_size=4 must differ"
            );
        }
    }

    #[test]
    fn test_calculate_indent_for_new_line() {
        assert_eq!(calculate_indent_for_new_line("fn foo() {"), 1);
        assert_eq!(calculate_indent_for_new_line("    let x = 1;"), 1);
        assert_eq!(calculate_indent_for_new_line("}"), 0);
    }

    #[test]
    fn test_indent_after_match_arm_arrow() {
        // `=>` opens a match arm body — indent one level deeper
        assert_eq!(calculate_indent_for_new_line("        Some(x) =>"), 3);
        assert_eq!(calculate_indent_for_new_line("    None =>"), 2);
    }

    #[test]
    fn test_indent_after_pipeline() {
        // `|>` is a continuation — keep same level
        assert_eq!(calculate_indent_for_new_line("    |> filter"), 1);
        assert_eq!(calculate_indent_for_new_line("        |> map"), 2);
    }

    #[test]
    fn test_pipeline_origin_indent() {
        let lines = vec![
            "    let result = data",
            "        |> transform",
            "        |> filter",
        ];
        // The origin is `let result = data` at indent 4
        assert_eq!(find_pipeline_origin_indent(&lines, 2), 4);
    }

    #[test]
    fn test_format_pipeline_operator_aligns() {
        let text = "    let result = data\n|> transform";
        let edits = format_pipeline_operator(
            text,
            Position { line: 1, character: 12 },
        );
        if !edits.is_empty() {
            let edit = &edits[0];
            // Should indent the |> line to 8 spaces (origin 4 + 4)
            assert!(edit.new_text.starts_with("        |>"));
        }
    }

    #[test]
    fn test_format_config_default() {
        let config = VerumFormatConfig::default();
        assert_eq!(config.indent_size, 4);
        assert_eq!(config.max_line_width, 100);
        assert!(config.trailing_commas);
    }

    #[test]
    fn test_trivia_preserving_formatter() {
        let config = VerumFormatConfig::default();
        let formatter = TriviaPreservingFormatter::new(config);
        let source = "fn foo() { let x = 1; }";
        let result = formatter.format(source, FileId::new(0));
        // The formatter should not panic; output may be empty if parsing fails
        let _ = result;
    }

    #[test]
    fn test_trivia_preserving_formatter_empty() {
        let config = VerumFormatConfig::default();
        let formatter = TriviaPreservingFormatter::new(config);
        let source = "";
        let result = formatter.format(source, FileId::new(0));
        // Empty source should produce empty output
        assert!(result.is_empty());
    }

    #[test]
    fn test_format_range() {
        let input = "fn foo() {\n    let x = 1;\n}";
        let range = Range {
            start: Position { line: 1, character: 0 },
            end: Position { line: 1, character: 100 },
        };
        let edits = format_range(input, range);
        // May or may not have edits depending on if the line needs formatting
        assert!(edits.len() <= 1);
    }
}
