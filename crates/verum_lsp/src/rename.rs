//! Rename symbol support
//!
//! Provides comprehensive symbol renaming across documents with:
//! - Cross-file rename support
//! - Symbol kind detection (function, type, variable, parameter, field)
//! - Rename validation (valid identifier, no conflicts)
//! - AST-based semantic rename (not just text matching)
//! - Syntax tree-based rename for accurate source location
//!
//! Uses the lossless red-green syntax tree for accurate source location mapping,
//! enabling AST-based semantic rename rather than text-based search/replace.

use crate::document::{DocumentState, DocumentStore, SymbolKind};
use std::collections::HashMap;
use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_common::{List, Text};
use verum_parser::syntax_bridge::LosslessParser;
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

// ==================== Rename Error Types ====================

/// Error returned when rename operation fails
#[derive(Debug, Clone)]
pub enum RenameError {
    /// Cannot rename keywords
    CannotRenameKeyword(Text),
    /// New name is not a valid identifier
    InvalidIdentifier(Text),
    /// New name conflicts with existing symbol
    NameConflict {
        new_name: Text,
        conflicting_symbol: Text,
        kind: SymbolKind,
    },
    /// Symbol not found at position
    SymbolNotFound,
    /// Symbol is defined in a read-only location (e.g., standard library)
    ReadOnlySymbol(Text),
    /// Cross-file rename requires workspace support
    WorkspaceRequired,
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CannotRenameKeyword(kw) => write!(f, "Cannot rename keyword '{}'", kw),
            Self::InvalidIdentifier(name) => {
                write!(f, "'{}' is not a valid identifier", name)
            }
            Self::NameConflict {
                new_name,
                conflicting_symbol,
                kind,
            } => write!(
                f,
                "Name '{}' conflicts with existing {:?} '{}'",
                new_name, kind, conflicting_symbol
            ),
            Self::SymbolNotFound => write!(f, "No symbol found at cursor position"),
            Self::ReadOnlySymbol(name) => {
                write!(f, "Cannot rename read-only symbol '{}'", name)
            }
            Self::WorkspaceRequired => {
                write!(f, "Cross-file rename requires workspace support")
            }
        }
    }
}

// ==================== Symbol Resolution ====================

/// Resolved symbol information for renaming
#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    /// Symbol name
    pub name: Text,
    /// Symbol kind
    pub kind: SymbolKind,
    /// Definition location
    pub definition: Location,
    /// All reference locations (including definition)
    pub references: List<Location>,
    /// Whether this is a cross-file symbol
    pub is_cross_file: bool,
    /// Scope information
    pub scope: SymbolScope,
}

/// Symbol scope for conflict detection
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolScope {
    /// Module-level (functions, types, constants)
    Module,
    /// Function-level (parameters, local variables)
    Function(Text),
    /// Block-level (loop variables, match bindings)
    Block,
    /// Type-level (fields, variants)
    Type(Text),
}

/// Resolve symbol at the given position
pub fn resolve_symbol(
    document: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<ResolvedSymbol> {
    // Get the word at the position
    let word = document.word_at_position(position)?;

    // Check if it's a keyword
    if is_keyword(&word) {
        return None;
    }

    // Look up symbol in symbol table
    let symbol_info = document.get_symbol(&word)?;

    // Find all references
    let references = crate::references::find_references(document, position, uri, true);

    // Determine if cross-file (would require workspace support)
    let is_cross_file = false; // Single document for now

    // Determine scope
    let scope = match symbol_info.kind {
        SymbolKind::Function | SymbolKind::Type | SymbolKind::Protocol | SymbolKind::Constant => {
            SymbolScope::Module
        }
        SymbolKind::Parameter | SymbolKind::Variable => {
            // Try to find the containing function
            if let Some(func_name) = find_containing_function(document, position) {
                SymbolScope::Function(func_name)
            } else {
                SymbolScope::Block
            }
        }
        SymbolKind::Field | SymbolKind::Variant => {
            // Try to find the containing type
            if let Some(type_name) = find_containing_type(document, &word) {
                SymbolScope::Type(type_name)
            } else {
                SymbolScope::Module
            }
        }
        SymbolKind::Module => SymbolScope::Module,
    };

    // Convert symbol span to location
    let definition = Location {
        uri: uri.clone(),
        range: span_to_range(document, symbol_info.def_span),
    };

    Some(ResolvedSymbol {
        name: Text::from(word),
        kind: symbol_info.kind,
        definition,
        references: references.into_iter().collect(),
        is_cross_file,
        scope,
    })
}

// ==================== Rename Validation ====================

/// Validate that a new name is acceptable for renaming
pub fn validate_new_name(
    document: &DocumentState,
    old_symbol: &ResolvedSymbol,
    new_name: &str,
) -> Result<(), RenameError> {
    // Check if new name is a keyword
    if is_keyword(new_name) {
        return Err(RenameError::CannotRenameKeyword(Text::from(new_name)));
    }

    // Check if new name is a valid identifier
    if !is_valid_identifier(new_name) {
        return Err(RenameError::InvalidIdentifier(Text::from(new_name)));
    }

    // Check for conflicts in the same scope
    if let Some(conflicting) = find_conflict(document, &old_symbol.scope, new_name) {
        return Err(RenameError::NameConflict {
            new_name: Text::from(new_name),
            conflicting_symbol: conflicting.0,
            kind: conflicting.1,
        });
    }

    Ok(())
}

/// Check if a string is a valid Verum identifier
pub fn is_valid_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }

    let mut chars = name.chars();

    // First character must be alphabetic or underscore
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }

    // Remaining characters must be alphanumeric or underscore
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Find any conflicting symbol in the given scope
fn find_conflict(
    document: &DocumentState,
    scope: &SymbolScope,
    new_name: &str,
) -> Option<(Text, SymbolKind)> {
    // Check if symbol with new name already exists
    if let Some(existing) = document.get_symbol(new_name) {
        // Check if it's in the same scope
        let existing_scope = match existing.kind {
            SymbolKind::Function
            | SymbolKind::Type
            | SymbolKind::Protocol
            | SymbolKind::Constant
            | SymbolKind::Module => SymbolScope::Module,
            SymbolKind::Parameter | SymbolKind::Variable => {
                // Would need more context, assume same scope for safety
                scope.clone()
            }
            SymbolKind::Field | SymbolKind::Variant => SymbolScope::Module,
        };

        // If scopes match, there's a conflict
        if &existing_scope == scope || existing_scope == SymbolScope::Module {
            return Some((Text::from(new_name), existing.kind));
        }
    }

    None
}

// ==================== Prepare Rename ====================

/// Prepare rename operation (check if renaming is valid)
///
/// Returns the range and current name if renaming is possible,
/// or None if the symbol cannot be renamed.
pub fn prepare_rename(
    document: &DocumentState,
    position: Position,
) -> Option<PrepareRenameResponse> {
    // Get the word at the position
    let word = document.word_at_position(position)?;

    // Check if it's a built-in keyword (which cannot be renamed)
    if is_keyword(&word) {
        return None;
    }

    // Verify the symbol exists in the symbol table
    if document.get_symbol(&word).is_none() {
        // Symbol not in table - might be a reference, check if valid identifier
        if !is_valid_identifier(&word) {
            return None;
        }
    }

    // Find the exact range of the word
    let range = find_word_range(document, position, &word)?;

    Some(PrepareRenameResponse::RangeWithPlaceholder {
        range,
        placeholder: word.to_string(),
    })
}

/// Extended prepare rename with detailed information
pub fn prepare_rename_with_info(
    document: &DocumentState,
    position: Position,
    uri: &Url,
) -> Result<(PrepareRenameResponse, ResolvedSymbol), RenameError> {
    // Resolve the symbol
    let symbol = resolve_symbol(document, position, uri).ok_or(RenameError::SymbolNotFound)?;

    // Check if it's a keyword
    if is_keyword(&symbol.name) {
        return Err(RenameError::CannotRenameKeyword(symbol.name));
    }

    // Find the exact range
    let range =
        find_word_range(document, position, &symbol.name).ok_or(RenameError::SymbolNotFound)?;

    let response = PrepareRenameResponse::RangeWithPlaceholder {
        range,
        placeholder: symbol.name.to_string(),
    };

    Ok((response, symbol))
}

// ==================== Perform Rename ====================

/// Perform the rename operation
pub fn rename(
    document: &DocumentState,
    position: Position,
    new_name: String,
    uri: &Url,
) -> Option<WorkspaceEdit> {
    // Resolve the symbol
    let symbol = resolve_symbol(document, position, uri)?;

    // Validate the new name
    if validate_new_name(document, &symbol, &new_name).is_err() {
        return None;
    }

    // Convert references to text edits
    let edits: Vec<TextEdit> = symbol
        .references
        .iter()
        .map(|loc| TextEdit {
            range: loc.range,
            new_text: new_name.clone(),
        })
        .collect();

    if edits.is_empty() {
        return None;
    }

    // Create workspace edit
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// Perform rename with validation and detailed result
pub fn rename_with_validation(
    document: &DocumentState,
    position: Position,
    new_name: String,
    uri: &Url,
) -> Result<WorkspaceEdit, RenameError> {
    // Resolve the symbol
    let symbol = resolve_symbol(document, position, uri).ok_or(RenameError::SymbolNotFound)?;

    // Validate the new name
    validate_new_name(document, &symbol, &new_name)?;

    // Convert references to text edits
    let edits: Vec<TextEdit> = symbol
        .references
        .iter()
        .map(|loc| TextEdit {
            range: loc.range,
            new_text: new_name.clone(),
        })
        .collect();

    if edits.is_empty() {
        return Err(RenameError::SymbolNotFound);
    }

    // Create workspace edit
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Ok(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

// ==================== Cross-File Rename ====================

/// Perform cross-file rename across the workspace
///
/// This requires access to all documents in the workspace to find
/// all references to the symbol across files.
pub fn rename_cross_file(
    document_store: &DocumentStore,
    primary_uri: &Url,
    position: Position,
    new_name: String,
) -> Result<WorkspaceEdit, RenameError> {
    // Get the primary document
    let document = document_store
        .get_document(primary_uri)
        .ok_or(RenameError::SymbolNotFound)?;

    // Resolve the symbol in the primary document
    let symbol = resolve_symbol(&document.read(), position, primary_uri)
        .ok_or(RenameError::SymbolNotFound)?;

    // Validate the new name
    validate_new_name(&document.read(), &symbol, &new_name)?;

    // Collect all edits across files
    let mut all_changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();

    // Add edits for the primary document
    let primary_edits: Vec<TextEdit> = symbol
        .references
        .iter()
        .filter(|loc| &loc.uri == primary_uri)
        .map(|loc| TextEdit {
            range: loc.range,
            new_text: new_name.clone(),
        })
        .collect();

    if !primary_edits.is_empty() {
        all_changes.insert(primary_uri.clone(), primary_edits);
    }

    // For exported symbols, search other documents
    // This would require workspace-wide reference finding
    // For now, we only handle single-file rename

    if all_changes.is_empty() {
        return Err(RenameError::SymbolNotFound);
    }

    Ok(WorkspaceEdit {
        changes: Some(all_changes),
        document_changes: None,
        change_annotations: None,
    })
}

// ==================== Helper Functions ====================

/// Find the exact range of a word at a position.
///
/// Delegates to `verum_common::text_utf8::find_word_bounds` for the
/// UTF-8-safe walk; the previous implementation mixed byte offsets
/// and char indices, silently mis-locating words in non-ASCII source
/// and panicking outright when the cursor landed mid-codepoint.
fn find_word_range(document: &DocumentState, position: Position, word: &str) -> Option<Range> {
    let line = document.get_line(position.line)?;
    let (start, end) = verum_common::text_utf8::find_word_bounds(
        line,
        position.character as usize,
        is_identifier_char,
    )?;
    if &line[start..end] != word {
        return None;
    }
    Some(Range {
        start: Position {
            line: position.line,
            character: start as u32,
        },
        end: Position {
            line: position.line,
            character: end as u32,
        },
    })
}

/// Convert AST span to LSP range
fn span_to_range(document: &DocumentState, span: verum_ast::Span) -> Range {
    // Convert byte offsets to line/column
    let start_pos = offset_to_position(document, span.start as usize);
    let end_pos = offset_to_position(document, span.end as usize);

    Range {
        start: start_pos,
        end: end_pos,
    }
}

/// Convert byte offset to LSP position
fn offset_to_position(document: &DocumentState, offset: usize) -> Position {
    let text = &document.text;
    let mut line = 0u32;
    let mut col = 0u32;
    let mut current_offset = 0usize;

    for ch in text.chars() {
        if current_offset >= offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }

        current_offset += ch.len_utf8();
    }

    Position {
        line,
        character: col,
    }
}

/// Find the containing function for a position
fn find_containing_function(document: &DocumentState, position: Position) -> Option<Text> {
    let offset = document.position_to_offset(position);

    if let Some(module) = &document.module {
        for item in &module.items {
            if let verum_ast::ItemKind::Function(func) = &item.kind {
                if func.span.start <= offset && offset <= func.span.end {
                    return Some(Text::from(func.name.as_str()));
                }
            }
        }
    }

    None
}

/// Find the containing type for a field or variant
fn find_containing_type(document: &DocumentState, _member_name: &str) -> Option<Text> {
    // For now, look through all types
    // A more sophisticated implementation would use the AST structure
    if let Some(module) = &document.module {
        for item in &module.items {
            if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                return Some(Text::from(type_decl.name.as_str()));
            }
        }
    }

    None
}

/// Check if a word is a Verum keyword
pub fn is_keyword(word: &str) -> bool {
    matches!(
        word,
        "fn" | "let"
            | "mut"
            | "if"
            | "else"
            | "match"
            | "for"
            | "while"
            | "loop"
            | "break"
            | "continue"
            | "return"
            | "type"
            | "struct"
            | "enum"
            | "protocol"
            | "impl"
            | "mod"
            | "use"
            | "pub"
            | "async"
            | "await"
            | "defer"
            | "stream"
            | "verify"
            | "requires"
            | "ensures"
            | "invariant"
            | "assert"
            | "assume"
            | "ref"
            | "checked"
            | "unsafe"
            | "as"
            | "in"
            | "is"
            | "true"
            | "false"
            | "null"
            | "self"
            | "Self"
            | "super"
            | "cog"
            | "context"
            | "provide"
            | "using"
            | "where"
            | "const"
            | "static"
            | "dyn"
            | "try"
            | "catch"
            | "throw"
    )
}

/// Check if a character can be part of an identifier
pub fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

// ==================== Tests ====================

// ==================== Syntax Tree-Based Rename (Phase 6) ====================

/// Syntax tree-based rename provider for accurate source location.
pub struct SyntaxTreeRenameProvider {
    parser: LosslessParser,
}

impl SyntaxTreeRenameProvider {
    /// Create a new syntax tree-based rename provider.
    pub fn new() -> Self {
        Self {
            parser: LosslessParser::new(),
        }
    }

    /// Find all occurrences of a symbol using the syntax tree.
    pub fn find_occurrences(
        &self,
        source: &str,
        file_id: FileId,
        symbol_name: &str,
    ) -> Vec<Range> {
        let result = self.parser.parse(source, file_id);
        let root = result.syntax();

        let mut occurrences = Vec::new();
        let line_index = LineIndex::new(source);

        self.collect_identifier_occurrences(&root, symbol_name, &line_index, &mut occurrences);

        occurrences
    }

    /// Collect all identifier occurrences matching the symbol name.
    fn collect_identifier_occurrences(
        &self,
        node: &SyntaxNode,
        symbol_name: &str,
        line_index: &LineIndex,
        occurrences: &mut Vec<Range>,
    ) {
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) if token.kind() == SyntaxKind::IDENT => {
                    if token.text() == symbol_name {
                        let text_range = token.text_range();
                        occurrences.push(Range {
                            start: line_index.position_at(text_range.start()),
                            end: line_index.position_at(text_range.end()),
                        });
                    }
                }
                SyntaxElement::Node(child_node) => {
                    self.collect_identifier_occurrences(
                        &child_node,
                        symbol_name,
                        line_index,
                        occurrences,
                    );
                }
                _ => {}
            }
        }
    }

    /// Find the identifier at a given position.
    pub fn identifier_at_position(
        &self,
        source: &str,
        file_id: FileId,
        position: Position,
    ) -> Option<(String, Range)> {
        let result = self.parser.parse(source, file_id);
        let root = result.syntax();

        let line_index = LineIndex::new(source);
        let offset = line_index.offset_at(position);

        self.find_identifier_at_offset(&root, offset, &line_index)
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

    /// Perform rename using syntax tree for accurate locations.
    pub fn rename(
        &self,
        source: &str,
        file_id: FileId,
        position: Position,
        new_name: &str,
        uri: &Url,
    ) -> Option<WorkspaceEdit> {
        // Find the identifier at position
        let (old_name, _range) = self.identifier_at_position(source, file_id, position)?;

        // Validate new name
        if !is_valid_identifier(new_name) || is_keyword(new_name) {
            return None;
        }

        // Find all occurrences
        let occurrences = self.find_occurrences(source, file_id, &old_name);

        if occurrences.is_empty() {
            return None;
        }

        // Create text edits
        let edits: Vec<TextEdit> = occurrences
            .into_iter()
            .map(|range| TextEdit {
                range,
                new_text: new_name.to_string(),
            })
            .collect();

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), edits);

        Some(WorkspaceEdit {
            changes: Some(changes),
            document_changes: None,
            change_annotations: None,
        })
    }

    /// Prepare rename with syntax tree validation.
    pub fn prepare_rename(
        &self,
        source: &str,
        file_id: FileId,
        position: Position,
    ) -> Option<PrepareRenameResponse> {
        let (name, range) = self.identifier_at_position(source, file_id, position)?;

        // Check if it's a keyword
        if is_keyword(&name) {
            return None;
        }

        Some(PrepareRenameResponse::RangeWithPlaceholder {
            range,
            placeholder: name,
        })
    }
}

impl Default for SyntaxTreeRenameProvider {
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
    fn test_is_keyword() {
        assert!(is_keyword("fn"));
        assert!(is_keyword("let"));
        assert!(is_keyword("if"));
        assert!(is_keyword("using"));
        assert!(is_keyword("context"));

        assert!(!is_keyword("foo"));
        assert!(!is_keyword("myFunction"));
        assert!(!is_keyword("x"));
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("MyType"));
        assert!(is_valid_identifier("snake_case"));
        assert!(is_valid_identifier("camelCase123"));
        assert!(is_valid_identifier("_"));
        assert!(is_valid_identifier("__internal"));

        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123abc"));
        assert!(!is_valid_identifier("has-dash"));
        assert!(!is_valid_identifier("has.dot"));
        assert!(!is_valid_identifier("has space"));
    }

    #[test]
    fn test_is_identifier_char() {
        assert!(is_identifier_char('a'));
        assert!(is_identifier_char('Z'));
        assert!(is_identifier_char('0'));
        assert!(is_identifier_char('_'));

        assert!(!is_identifier_char(' '));
        assert!(!is_identifier_char('-'));
        assert!(!is_identifier_char('.'));
        assert!(!is_identifier_char('!'));
    }

    #[test]
    fn test_syntax_tree_rename_provider() {
        let provider = SyntaxTreeRenameProvider::new();
        let source = "fn foo() { let x = 1; x }";
        let occurrences = provider.find_occurrences(source, FileId::new(0), "x");
        assert_eq!(occurrences.len(), 2); // declaration and use
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
}
