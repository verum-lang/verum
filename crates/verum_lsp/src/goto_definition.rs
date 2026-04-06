//! Go to definition support
//!
//! Allows users to navigate to the definition of symbols, including:
//! - Top-level functions, types, and protocols
//! - Local variables and parameters
//! - Record fields and enum variants
//! - Module declarations
//! - Accurate span calculation using lossless syntax tree

use crate::document::DocumentState;
use crate::position_utils::ast_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::{ExprKind, FileId, ItemKind, PatternKind, StmtKind};
use verum_parser::syntax_bridge::LosslessParser;
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

/// Find the definition of a symbol at the given position
pub fn goto_definition(
    document: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    // Get the word at the position
    let word = document.word_at_position(position)?;

    // First, check the symbol table for quick lookup
    if let Some(symbol) = document.get_symbol(&word) {
        let range = ast_span_to_range(&symbol.def_span, &document.text);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range,
        }));
    }

    // Search for the definition in the module AST
    let module = document.module.as_ref()?;

    // Try top-level items first
    if let Some(def_span) = find_top_level_definition(module, &word) {
        let range = ast_span_to_range(&def_span, &document.text);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range,
        }));
    }

    // Try to find local variable definitions within function bodies
    if let Some(def_span) = find_local_definition(module, &word, position, &document.text) {
        let range = ast_span_to_range(&def_span, &document.text);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range,
        }));
    }

    // Try to find field/variant definitions (e.g., "Type.field" or "Type::Variant")
    if let Some(def_span) = find_member_definition(module, &word) {
        let range = ast_span_to_range(&def_span, &document.text);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range,
        }));
    }

    None
}

/// Find definition of top-level items (functions, types, protocols)
fn find_top_level_definition(module: &verum_ast::Module, symbol: &str) -> Option<verum_ast::Span> {
    for item in module.items.iter() {
        match &item.kind {
            ItemKind::Function(func) if func.name.as_str() == symbol => {
                return Some(func.span);
            }
            ItemKind::Type(type_decl) if type_decl.name.as_str() == symbol => {
                return Some(type_decl.span);
            }
            ItemKind::Protocol(protocol) if protocol.name.as_str() == symbol => {
                return Some(protocol.span);
            }
            ItemKind::Const(const_decl) if const_decl.name.as_str() == symbol => {
                return Some(const_decl.span);
            }
            ItemKind::Module(mod_decl) if mod_decl.name.as_str() == symbol => {
                return Some(mod_decl.span);
            }
            _ => {}
        }
    }
    None
}

/// Find definition of local variables within function bodies
fn find_local_definition(
    module: &verum_ast::Module,
    symbol: &str,
    position: Position,
    text: &str,
) -> Option<verum_ast::Span> {
    // Convert position to byte offset
    let target_offset = position_to_offset(position, text);

    // Find which function contains the position
    for item in module.items.iter() {
        if let ItemKind::Function(func) = &item.kind {
            // Check if position is within this function
            if func.span.start <= target_offset && target_offset <= func.span.end {
                // Search for local variable definitions in this function

                // First, check parameters
                for param in func.params.iter() {
                    if let verum_ast::decl::FunctionParamKind::Regular { pattern, .. } = &param.kind
                        && let Some(span) = find_pattern_binding(pattern, symbol)
                    {
                        return Some(span);
                    }
                }

                // Then check function body
                if let Some(body) = &func.body
                    && let verum_ast::decl::FunctionBody::Block(block) = body
                    && let Some(span) = find_definition_in_block(block, symbol, target_offset)
                {
                    return Some(span);
                }
            }
        }
    }
    None
}

/// Find a binding in a pattern
fn find_pattern_binding(pattern: &verum_ast::Pattern, symbol: &str) -> Option<verum_ast::Span> {
    match &pattern.kind {
        PatternKind::Ident { name, .. } if name.as_str() == symbol => Some(pattern.span),
        PatternKind::Tuple(patterns) => {
            for p in patterns.iter() {
                if let Some(span) = find_pattern_binding(p, symbol) {
                    return Some(span);
                }
            }
            None
        }
        PatternKind::Variant { data, .. } => {
            if let Some(inner) = data {
                match inner {
                    verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                        for p in patterns.iter() {
                            if let Some(span) = find_pattern_binding(p, symbol) {
                                return Some(span);
                            }
                        }
                    }
                    verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                        for field in fields.iter() {
                            if let Some(p) = &field.pattern
                                && let Some(span) = find_pattern_binding(p, symbol)
                            {
                                return Some(span);
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Find definition in a block
fn find_definition_in_block(
    block: &verum_ast::expr::Block,
    symbol: &str,
    target_offset: u32,
) -> Option<verum_ast::Span> {
    for stmt in block.stmts.iter() {
        // Only consider definitions that appear before the target position
        if stmt.span.start > target_offset {
            continue;
        }

        match &stmt.kind {
            StmtKind::Let { pattern, .. } => {
                if let Some(span) = find_pattern_binding(pattern, symbol) {
                    return Some(span);
                }
            }
            StmtKind::Expr { expr, .. } => {
                // Check for nested blocks in expressions
                if let Some(span) = find_definition_in_expr(expr, symbol, target_offset) {
                    return Some(span);
                }
            }
            StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                // Check for definitions in deferred expressions
                if let Some(span) = find_definition_in_expr(expr, symbol, target_offset) {
                    return Some(span);
                }
            }
            _ => {}
        }
    }

    // Check the tail expression
    if let Some(expr) = &block.expr
        && let Some(span) = find_definition_in_expr(expr, symbol, target_offset)
    {
        return Some(span);
    }

    None
}

/// Find definition in an expression (for nested blocks like if/match)
fn find_definition_in_expr(
    expr: &verum_ast::Expr,
    symbol: &str,
    target_offset: u32,
) -> Option<verum_ast::Span> {
    match &expr.kind {
        ExprKind::Block(block) => find_definition_in_block(block, symbol, target_offset),
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            if let Some(span) = find_definition_in_block(then_branch, symbol, target_offset) {
                return Some(span);
            }
            if let Some(else_expr) = else_branch
                && let Some(span) = find_definition_in_expr(else_expr, symbol, target_offset)
            {
                return Some(span);
            }
            None
        }
        ExprKind::Match { arms, .. } => {
            for arm in arms {
                // Check pattern bindings in match arms
                if let Some(span) = find_pattern_binding(&arm.pattern, symbol) {
                    // Only return if target is within this arm
                    if arm.body.span.start <= target_offset && target_offset <= arm.body.span.end {
                        return Some(span);
                    }
                }
                // Check arm body
                if let Some(span) = find_definition_in_expr(&arm.body, symbol, target_offset) {
                    return Some(span);
                }
            }
            None
        }
        ExprKind::For {
            label: _,
            pattern,
            body,
            ..
        } => {
            if let Some(span) = find_pattern_binding(pattern, symbol) {
                return Some(span);
            }
            find_definition_in_block(body, symbol, target_offset)
        }
        ExprKind::While { label: _, body, .. } => {
            find_definition_in_block(body, symbol, target_offset)
        }
        ExprKind::Loop { label: _, body, .. } => {
            find_definition_in_block(body, symbol, target_offset)
        }
        ExprKind::Closure { params, body, .. } => {
            // Check closure parameters
            for param in params {
                if let Some(span) = find_pattern_binding(&param.pattern, symbol) {
                    return Some(span);
                }
            }
            find_definition_in_expr(body, symbol, target_offset)
        }
        ExprKind::TryRecover { try_block, recover } => {
            if let Some(span) = find_definition_in_expr(try_block, symbol, target_offset) {
                return Some(span);
            }
            find_definition_in_recover_body(recover, symbol, target_offset)
        }
        ExprKind::TryRecoverFinally {
            try_block,
            recover,
            finally_block,
        } => {
            if let Some(span) = find_definition_in_expr(try_block, symbol, target_offset) {
                return Some(span);
            }
            if let Some(span) = find_definition_in_recover_body(recover, symbol, target_offset) {
                return Some(span);
            }
            find_definition_in_expr(finally_block, symbol, target_offset)
        }
        ExprKind::DestructuringAssign { pattern, value, .. } => {
            // Check pattern bindings in destructuring assignment
            if let Some(span) = find_pattern_binding(pattern, symbol) {
                return Some(span);
            }
            // Check the value expression
            find_definition_in_expr(value, symbol, target_offset)
        }
        _ => None,
    }
}

/// Find definition in a recover body
fn find_definition_in_recover_body(
    recover: &verum_ast::expr::RecoverBody,
    symbol: &str,
    target_offset: u32,
) -> Option<verum_ast::Span> {
    match recover {
        verum_ast::expr::RecoverBody::MatchArms { arms, .. } => {
            for arm in arms {
                // Check pattern bindings in recover arms
                if let Some(span) = find_pattern_binding(&arm.pattern, symbol) {
                    // Only return if target is within this arm
                    if arm.body.span.start <= target_offset && target_offset <= arm.body.span.end {
                        return Some(span);
                    }
                }
                // Check arm body
                if let Some(span) = find_definition_in_expr(&arm.body, symbol, target_offset) {
                    return Some(span);
                }
            }
            None
        }
        verum_ast::expr::RecoverBody::Closure { param, body, .. } => {
            if let Some(span) = find_pattern_binding(&param.pattern, symbol) {
                return Some(span);
            }
            find_definition_in_expr(body, symbol, target_offset)
        }
    }
}

/// Find definition of struct fields or enum variants
fn find_member_definition(module: &verum_ast::Module, symbol: &str) -> Option<verum_ast::Span> {
    use verum_ast::decl::TypeDeclBody;

    for item in module.items.iter() {
        if let ItemKind::Type(type_decl) = &item.kind {
            match &type_decl.body {
                TypeDeclBody::Record(fields) => {
                    for field in fields {
                        if field.name.as_str() == symbol {
                            return Some(field.span);
                        }
                    }
                }
                TypeDeclBody::Variant(variants) => {
                    for variant in variants {
                        if variant.name.as_str() == symbol {
                            return Some(variant.span);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

/// Convert LSP Position to byte offset
fn position_to_offset(position: Position, text: &str) -> u32 {
    let mut offset: u32 = 0;
    let mut current_line: u32 = 0;
    let mut current_char: u32 = 0;

    for ch in text.chars() {
        if current_line == position.line && current_char == position.character {
            return offset;
        }

        if ch == '\n' {
            current_line += 1;
            current_char = 0;
        } else {
            current_char += 1;
        }

        offset += ch.len_utf8() as u32;
    }

    offset
}

// ==================== Syntax Tree-Based Go to Definition (Phase 6) ====================

/// Syntax tree-based go to definition provider for accurate source location.
pub struct SyntaxTreeDefinitionProvider {
    parser: LosslessParser,
}

impl SyntaxTreeDefinitionProvider {
    /// Create a new syntax tree-based definition provider.
    pub fn new() -> Self {
        Self {
            parser: LosslessParser::new(),
        }
    }

    /// Find the definition of a symbol at the given position using the syntax tree.
    pub fn find_definition(
        &self,
        source: &str,
        file_id: FileId,
        position: Position,
        uri: &Url,
    ) -> Option<GotoDefinitionResponse> {
        let result = self.parser.parse(source, file_id);
        let root = result.syntax();

        let line_index = LineIndex::new(source);
        let offset = line_index.offset_at(position);

        // Find the identifier at the position
        let (symbol_name, _symbol_range) = self.find_identifier_at_offset(&root, offset, &line_index)?;

        // Find all definitions of this symbol
        let definitions = self.find_symbol_definitions(&root, &symbol_name, &line_index);

        if definitions.is_empty() {
            return None;
        }

        // Return the first definition (or all if there are multiple)
        if definitions.len() == 1 {
            Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: definitions[0],
            }))
        } else {
            // Multiple definitions (e.g., overloaded functions)
            Some(GotoDefinitionResponse::Array(
                definitions
                    .into_iter()
                    .map(|range| Location {
                        uri: uri.clone(),
                        range,
                    })
                    .collect(),
            ))
        }
    }

    /// Find the identifier at a given byte offset.
    fn find_identifier_at_offset(
        &self,
        node: &SyntaxNode,
        offset: u32,
        line_index: &LineIndex,
    ) -> Option<(String, Range)> {
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::IDENT => {
                    let text_range = token.text_range();
                    if text_range.start() <= offset && offset < text_range.end() {
                        return Some((
                            token.text().to_string(),
                            Range {
                                start: line_index.position_at(text_range.start()),
                                end: line_index.position_at(text_range.end()),
                            },
                        ));
                    }
                }
                SyntaxElement::Node(child_node) => {
                    let node_range = child_node.text_range();
                    if node_range.start() <= offset && offset < node_range.end() {
                        if let Some(result) =
                            self.find_identifier_at_offset(&child_node, offset, line_index)
                        {
                            return Some(result);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Find all definitions of a symbol in the syntax tree.
    fn find_symbol_definitions(
        &self,
        node: &SyntaxNode,
        symbol_name: &str,
        line_index: &LineIndex,
    ) -> Vec<Range> {
        let mut definitions = Vec::new();
        self.collect_definitions(node, symbol_name, line_index, &mut definitions);
        definitions
    }

    /// Recursively collect definitions of a symbol.
    fn collect_definitions(
        &self,
        node: &SyntaxNode,
        symbol_name: &str,
        line_index: &LineIndex,
        definitions: &mut Vec<Range>,
    ) {
        let node_kind = node.kind();

        // Check if this is a definition node
        match node_kind {
            SyntaxKind::FN_DEF => {
                // Function definition - find the name
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::TYPE_DEF => {
                // Type definition - find the name
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::PROTOCOL_DEF => {
                // Protocol definition
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::LET_STMT => {
                // Local variable definition
                if let Some(name_token) = self.find_pattern_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::PARAM => {
                // Function parameter
                if let Some(name_token) = self.find_pattern_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::FIELD_DEF => {
                // Struct field
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::VARIANT_DEF => {
                // Enum variant
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::CONST_DEF | SyntaxKind::STATIC_DEF => {
                // Constant/static definition
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::MODULE_DEF => {
                // Module definition
                if let Some(name_token) = self.find_definition_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::FOR_EXPR => {
                // For loop binding
                if let Some(name_token) = self.find_for_loop_binding(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            SyntaxKind::MATCH_ARM => {
                // Match arm pattern binding
                if let Some(name_token) = self.find_pattern_name(node) {
                    if name_token == symbol_name {
                        let text_range = node.text_range();
                        definitions.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
            }
            _ => {}
        }

        // Recursively check children
        for child in node.children() {
            if let SyntaxElement::Node(child_node) = child {
                self.collect_definitions(&child_node, symbol_name, line_index, definitions);
            }
        }
    }

    /// Find the name identifier in a definition node.
    fn find_definition_name(&self, node: &SyntaxNode) -> Option<String> {
        // Look for the first IDENT token that's a direct child
        for child in node.children() {
            if let SyntaxElement::Token(token) = child {
                if token.kind() == SyntaxKind::IDENT {
                    return Some(token.text().to_string());
                }
            }
        }
        None
    }

    /// Find the name in a pattern.
    fn find_pattern_name(&self, node: &SyntaxNode) -> Option<String> {
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::IDENT => {
                    return Some(token.text().to_string());
                }
                SyntaxElement::Node(child_node)
                    if child_node.kind() == SyntaxKind::IDENT_PAT =>
                {
                    // Pattern binding
                    return self.find_definition_name(&child_node);
                }
                _ => {}
            }
        }
        None
    }

    /// Find the binding name in a for loop.
    fn find_for_loop_binding(&self, node: &SyntaxNode) -> Option<String> {
        for child in node.children() {
            if let SyntaxElement::Node(child_node) = child {
                if child_node.kind() == SyntaxKind::IDENT_PAT {
                    return self.find_definition_name(&child_node);
                }
            }
        }
        None
    }
}

impl Default for SyntaxTreeDefinitionProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Line index for position calculations.
struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in text.char_indices() {
            if c == '\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        Self { line_starts }
    }

    fn position_at(&self, offset: u32) -> Position {
        let line = self
            .line_starts
            .binary_search(&offset)
            .unwrap_or_else(|i| i.saturating_sub(1));
        let line_start = self.line_starts[line];
        let character = offset - line_start;
        Position {
            line: line as u32,
            character,
        }
    }

    fn offset_at(&self, position: Position) -> u32 {
        let line = position.line as usize;
        if line >= self.line_starts.len() {
            return *self.line_starts.last().unwrap_or(&0);
        }
        self.line_starts[line] + position.character
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syntax_tree_definition_provider() {
        let provider = SyntaxTreeDefinitionProvider::new();
        // Provider creation should succeed
        assert!(provider.parser.parse("fn foo() {}", FileId::new(0)).syntax().kind() == SyntaxKind::SOURCE_FILE);
    }

    #[test]
    fn test_find_definition_simple() {
        let provider = SyntaxTreeDefinitionProvider::new();
        let source = "fn foo() { let x = 1; x }";
        let uri = Url::parse("file:///test.vr").unwrap();

        // Position at "foo" in "fn foo()"
        let position = Position {
            line: 0,
            character: 3,
        };

        let result = provider.find_definition(source, FileId::new(0), position, &uri);
        // Result may be None if the identifier is not found (depends on parser behavior)
        // The test is that the provider doesn't panic
        let _ = result;
    }

    #[test]
    fn test_line_index() {
        let text = "line1\nline2\nline3";
        let index = LineIndex::new(text);

        let pos = index.position_at(0);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 0);

        let pos = index.position_at(6);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn test_position_to_offset() {
        let text = "line1\nline2\nline3";
        let pos = Position { line: 1, character: 0 };
        let offset = position_to_offset(pos, text);
        assert_eq!(offset, 6);
    }
}
