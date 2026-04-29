//! Diagnostic conversion and publishing with incremental support
//!
//! Converts Verum compiler diagnostics to LSP format and publishes them to the client.
//! This module provides comprehensive diagnostic conversion with:
//! - Severity mapping (Error/Warning/Info/Hint)
//! - Related information for context
//! - Code action suggestions (quick fixes)
//! - Diagnostic tags (deprecated, unnecessary, etc.)
//! - Links to error documentation
//! - Incremental diagnostics from ERROR nodes in syntax tree

use crate::position_utils::verum_span_to_range;
use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_diagnostics::{Diagnostic as VerumDiagnostic, Severity};
use verum_parser::syntax_bridge::LosslessParser;
use verum_common::List;
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

// ==================== Core Diagnostic Conversion ====================

/// Convert a Verum diagnostic to an LSP diagnostic
pub fn to_lsp_diagnostic(diagnostic: &VerumDiagnostic, text: &str, uri: &Url) -> LspDiagnostic {
    // Get primary span from the diagnostic
    let primary_span = diagnostic.primary_span();
    let range = if let Some(span) = primary_span {
        verum_span_to_range(span, text)
    } else {
        // Default to start of file if no span
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 0,
                character: 0,
            },
        }
    };

    let severity = match diagnostic.severity() {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Note => DiagnosticSeverity::INFORMATION,
        Severity::Help => DiagnosticSeverity::HINT,
    };

    let mut lsp_diagnostic = LspDiagnostic {
        range,
        severity: Some(severity),
        code: diagnostic
            .code()
            .map(|c| NumberOrString::String(c.to_string())),
        source: Some("verum".to_string()),
        message: diagnostic.message().to_string(),
        related_information: None,
        tags: None,
        code_description: None,
        data: None,
    };

    // Add related information from secondary labels
    let secondary_labels = diagnostic.secondary_labels();
    if !secondary_labels.is_empty() {
        let related: Vec<DiagnosticRelatedInformation> = secondary_labels
            .iter()
            .filter_map(|label| {
                Some(DiagnosticRelatedInformation {
                    location: Location {
                        uri: uri.clone(),
                        range: verum_span_to_range(&label.span, text),
                    },
                    message: label.message.to_string(),
                })
            })
            .collect();

        if !related.is_empty() {
            lsp_diagnostic.related_information = Some(related);
        }
    }

    // Add code description link if we have an error code
    if let Some(code) = diagnostic.code() {
        lsp_diagnostic.code_description = Some(CodeDescription {
            href: Url::parse(&format!("https://verum-lang.org/errors/{}", code))
                .or_else(|_| Url::parse("https://verum-lang.org/errors"))
                .expect("static fallback URL is always valid"),
        });
    }

    // Add diagnostic tags based on message content
    let tags = infer_diagnostic_tags(diagnostic.message());
    if !tags.is_empty() {
        lsp_diagnostic.tags = Some(tags);
    }

    lsp_diagnostic
}

/// Infer diagnostic tags from the diagnostic message
fn infer_diagnostic_tags(message: &str) -> Vec<DiagnosticTag> {
    let mut tags = Vec::new();
    let lower = message.to_lowercase();

    if lower.contains("deprecated") {
        tags.push(DiagnosticTag::DEPRECATED);
    }

    if lower.contains("unused") || lower.contains("never used") {
        tags.push(DiagnosticTag::UNNECESSARY);
    }

    tags
}

/// Convert Verum diagnostics to LSP diagnostics
pub fn convert_diagnostics(
    diagnostics: List<VerumDiagnostic>,
    text: &str,
    uri: &Url,
) -> Vec<LspDiagnostic> {
    diagnostics
        .iter()
        .map(|d| to_lsp_diagnostic(d, text, uri))
        .collect()
}

// ==================== Incremental Diagnostics from Syntax Tree ====================

/// Provider for incremental diagnostics based on syntax tree ERROR nodes.
pub struct IncrementalDiagnosticsProvider {
    /// Cache of previous diagnostics for delta computation
    previous_diagnostics: Vec<LspDiagnostic>,
}

impl IncrementalDiagnosticsProvider {
    /// Create a new incremental diagnostics provider.
    pub fn new() -> Self {
        Self {
            previous_diagnostics: Vec::new(),
        }
    }

    /// Extract diagnostics from ERROR nodes in the syntax tree.
    pub fn diagnostics_from_syntax_tree(
        &mut self,
        source: &str,
        file_id: FileId,
        uri: &Url,
    ) -> Vec<LspDiagnostic> {
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        let root = result.syntax();

        let mut diagnostics = Vec::new();
        let line_index = LineIndex::new(source);

        self.collect_error_diagnostics(&root, &line_index, uri, &mut diagnostics);

        // Store for delta computation
        self.previous_diagnostics = diagnostics.clone();

        diagnostics
    }

    /// Compute incremental diagnostic update after an edit.
    pub fn compute_incremental_update(
        &mut self,
        source: &str,
        file_id: FileId,
        uri: &Url,
        edit_range: Range,
    ) -> DiagnosticUpdate {
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        let root = result.syntax();

        let mut new_diagnostics = Vec::new();
        let line_index = LineIndex::new(source);

        self.collect_error_diagnostics(&root, &line_index, uri, &mut new_diagnostics);

        // Compute changes
        let added: Vec<_> = new_diagnostics
            .iter()
            .filter(|d| !self.previous_diagnostics.iter().any(|p| diagnostics_equal(p, d)))
            .cloned()
            .collect();

        let removed: Vec<_> = self
            .previous_diagnostics
            .iter()
            .filter(|p| {
                // Remove diagnostics in the edit range or not in new set
                ranges_overlap(&p.range, &edit_range)
                    || !new_diagnostics.iter().any(|d| diagnostics_equal(p, d))
            })
            .cloned()
            .collect();

        // Update cache
        self.previous_diagnostics = new_diagnostics.clone();

        DiagnosticUpdate {
            all: new_diagnostics,
            added,
            removed,
        }
    }

    /// Collect diagnostics from ERROR nodes in the syntax tree.
    fn collect_error_diagnostics(
        &self,
        node: &SyntaxNode,
        line_index: &LineIndex,
        uri: &Url,
        diagnostics: &mut Vec<LspDiagnostic>,
    ) {
        // Check if this node is an ERROR node
        if node.kind() == SyntaxKind::ERROR {
            let text_range = node.text_range();
            let range = Range {
                start: line_index.position_at(text_range.start()),
                end: line_index.position_at(text_range.end()),
            };

            // Analyze the error to provide a meaningful message
            let (message, code) = self.analyze_error_node(node);

            let diagnostic = LspDiagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(code)),
                source: Some("verum-parser".to_string()),
                message,
                related_information: None,
                tags: None,
                code_description: Some(CodeDescription {
                    href: Url::parse("https://verum-lang.org/errors/syntax")
                        .unwrap_or_else(|_| uri.clone()),
                }),
                data: None,
            };

            diagnostics.push(diagnostic);
        }

        // Recursively check children
        for child in node.children() {
            if let SyntaxElement::Node(child_node) = child {
                self.collect_error_diagnostics(&child_node, line_index, uri, diagnostics);
            }
        }
    }

    /// Analyze an ERROR node to determine the type of syntax error.
    fn analyze_error_node(&self, node: &SyntaxNode) -> (String, String) {
        let text = node.text();
        let parent_kind = node.parent().map(|p| p.kind());

        // Check what tokens are in the error node
        let tokens: Vec<_> = node.child_tokens().collect();

        // Analyze based on context and content
        let (message, code) = if text.is_empty() {
            // Empty error node - something is missing
            match parent_kind {
                Some(SyntaxKind::FN_DEF) => ("Expected function body or semicolon".to_string(), "E0001".to_string()),
                Some(SyntaxKind::TYPE_DEF) => ("Expected type definition body".to_string(), "E0002".to_string()),
                Some(SyntaxKind::BLOCK) => ("Expected expression or statement".to_string(), "E0003".to_string()),
                Some(SyntaxKind::PARAM_LIST) => ("Expected parameter or ')'".to_string(), "E0004".to_string()),
                _ => ("Unexpected end of input".to_string(), "E0000".to_string()),
            }
        } else if tokens.iter().any(|t| t.kind() == SyntaxKind::L_BRACE) && !tokens.iter().any(|t| t.kind() == SyntaxKind::R_BRACE) {
            ("Missing closing brace '}'".to_string(), "E0010".to_string())
        } else if tokens.iter().any(|t| t.kind() == SyntaxKind::L_PAREN) && !tokens.iter().any(|t| t.kind() == SyntaxKind::R_PAREN) {
            ("Missing closing parenthesis ')'".to_string(), "E0011".to_string())
        } else if tokens.iter().any(|t| t.kind() == SyntaxKind::L_BRACKET) && !tokens.iter().any(|t| t.kind() == SyntaxKind::R_BRACKET) {
            ("Missing closing bracket ']'".to_string(), "E0012".to_string())
        } else if text.ends_with('{') || text.ends_with('(') || text.ends_with('[') {
            ("Unclosed delimiter".to_string(), "E0013".to_string())
        } else if !text.ends_with(';') && self.looks_like_statement(node) {
            ("Missing semicolon".to_string(), "E0020".to_string())
        } else {
            // Generic syntax error.  Truncate by characters, not raw
            // bytes — uses the UTF-8-safe truncate from verum_common
            // so combining marks / emoji / CJK in user source can't
            // crash the diagnostic generator.
            let text_string = text.to_string();
            let preview = verum_common::text_utf8::truncate_chars(&text_string, 20);
            let preview = if preview.len() < text_string.len() {
                format!("{}...", preview)
            } else {
                preview.to_string()
            };
            (format!("Syntax error near '{}'", preview), "E0099".to_string())
        };

        (message, code)
    }

    /// Check if the error node looks like a statement that needs a semicolon.
    fn looks_like_statement(&self, node: &SyntaxNode) -> bool {
        let parent = node.parent();
        if let Some(p) = parent {
            matches!(
                p.kind(),
                SyntaxKind::BLOCK | SyntaxKind::SOURCE_FILE | SyntaxKind::MODULE_DEF
            )
        } else {
            false
        }
    }
}

impl Default for IncrementalDiagnosticsProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of an incremental diagnostic update.
#[derive(Debug, Clone)]
pub struct DiagnosticUpdate {
    /// All current diagnostics
    pub all: Vec<LspDiagnostic>,
    /// Diagnostics added since last update
    pub added: Vec<LspDiagnostic>,
    /// Diagnostics removed since last update
    pub removed: Vec<LspDiagnostic>,
}

/// Check if two diagnostics are equal (for delta computation).
fn diagnostics_equal(a: &LspDiagnostic, b: &LspDiagnostic) -> bool {
    a.range == b.range && a.message == b.message && a.code == b.code
}

/// Check if two ranges overlap.
fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character < b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character < a.start.character))
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
}

// ==================== Quick Fix Generation ====================

/// Enhanced diagnostic with quick fix suggestions
#[derive(Debug, Clone)]
pub struct DiagnosticWithFix {
    pub diagnostic: LspDiagnostic,
    pub quick_fixes: Vec<CodeAction>,
}

/// Generate quick fixes for common diagnostic patterns
pub fn generate_quick_fixes(
    diagnostic: &VerumDiagnostic,
    text: &str,
    uri: &Url,
) -> Vec<CodeAction> {
    let mut fixes = Vec::new();
    let message = diagnostic.message().to_lowercase();

    // Quick fix: Import missing symbol
    if (message.contains("not found in scope") || message.contains("cannot find"))
        && let Some(symbol) = extract_symbol_name(&message)
    {
        fixes.push(create_import_fix(symbol, uri, text));
    }

    // Quick fix: Add missing type annotation
    if (message.contains("type annotation needed") || message.contains("cannot infer type"))
        && let Some(span) = diagnostic.primary_span()
    {
        fixes.push(create_type_annotation_fix(span, uri, text));
    }

    // Quick fix: Remove unused variable
    if message.contains("unused variable")
        && let Some(span) = diagnostic.primary_span()
    {
        fixes.push(create_remove_unused_fix(span, uri, text));
    }

    // Quick fix: Add missing return
    if message.contains("missing return")
        && let Some(span) = diagnostic.primary_span()
    {
        fixes.push(create_add_return_fix(span, uri, text));
    }

    fixes
}

/// Generate quick fixes from ERROR node diagnostics
pub fn generate_error_node_fixes(
    diagnostic: &LspDiagnostic,
    _text: &str,
    uri: &Url,
) -> Vec<CodeAction> {
    let mut fixes = Vec::new();
    let code = diagnostic.code.as_ref().and_then(|c| match c {
        NumberOrString::String(s) => Some(s.as_str()),
        NumberOrString::Number(_) => None,
    });

    match code {
        Some("E0010") => {
            // Missing closing brace
            fixes.push(CodeAction {
                title: "Add closing brace '}'".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some({
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(
                            uri.clone(),
                            vec![TextEdit {
                                range: Range {
                                    start: diagnostic.range.end,
                                    end: diagnostic.range.end,
                                },
                                new_text: "}".to_string(),
                            }],
                        );
                        changes
                    }),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            });
        }
        Some("E0011") => {
            // Missing closing parenthesis
            fixes.push(CodeAction {
                title: "Add closing parenthesis ')'".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some({
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(
                            uri.clone(),
                            vec![TextEdit {
                                range: Range {
                                    start: diagnostic.range.end,
                                    end: diagnostic.range.end,
                                },
                                new_text: ")".to_string(),
                            }],
                        );
                        changes
                    }),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            });
        }
        Some("E0012") => {
            // Missing closing bracket
            fixes.push(CodeAction {
                title: "Add closing bracket ']'".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some({
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(
                            uri.clone(),
                            vec![TextEdit {
                                range: Range {
                                    start: diagnostic.range.end,
                                    end: diagnostic.range.end,
                                },
                                new_text: "]".to_string(),
                            }],
                        );
                        changes
                    }),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            });
        }
        Some("E0020") => {
            // Missing semicolon
            fixes.push(CodeAction {
                title: "Add semicolon".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some({
                        let mut changes = std::collections::HashMap::new();
                        changes.insert(
                            uri.clone(),
                            vec![TextEdit {
                                range: Range {
                                    start: diagnostic.range.end,
                                    end: diagnostic.range.end,
                                },
                                new_text: ";".to_string(),
                            }],
                        );
                        changes
                    }),
                    document_changes: None,
                    change_annotations: None,
                }),
                command: None,
                is_preferred: Some(true),
                disabled: None,
                data: None,
            });
        }
        _ => {}
    }

    fixes
}

/// Extract symbol name from error message
fn extract_symbol_name(message: &str) -> Option<String> {
    // Look for patterns like "symbol 'Foo' not found"
    if let Some(start) = message.find('\'')
        && let Some(end) = message[start + 1..].find('\'')
    {
        return Some(message[start + 1..start + 1 + end].to_string());
    }

    // Look for patterns like "`Foo` not found"
    if let Some(start) = message.find('`')
        && let Some(end) = message[start + 1..].find('`')
    {
        return Some(message[start + 1..start + 1 + end].to_string());
    }

    None
}

/// Create a quick fix to import a missing symbol
fn create_import_fix(symbol: String, uri: &Url, _text: &str) -> CodeAction {
    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = std::collections::HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    new_text: format!("use {};\n", symbol),
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: format!("Import `{}`", symbol),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    }
}

/// Create a quick fix to add type annotation
fn create_type_annotation_fix(span: &verum_diagnostics::Span, uri: &Url, text: &str) -> CodeAction {
    let range = verum_span_to_range(span, text);

    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = std::collections::HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: range.end,
                        end: range.end,
                    },
                    new_text: ": Type".to_string(),
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: "Add type annotation".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

/// Create a quick fix to remove unused variable
fn create_remove_unused_fix(span: &verum_diagnostics::Span, uri: &Url, text: &str) -> CodeAction {
    let range = verum_span_to_range(span, text);

    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = std::collections::HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range,
                    new_text: String::new(),
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: "Remove unused variable".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

/// Create a quick fix to add missing return statement
fn create_add_return_fix(span: &verum_diagnostics::Span, uri: &Url, text: &str) -> CodeAction {
    let range = verum_span_to_range(span, text);

    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = std::collections::HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: range.end,
                        end: range.end,
                    },
                    new_text: "\nreturn value;".to_string(),
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: "Add return statement".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

// Re-export LSP's Diagnostic with a different name to avoid confusion
pub use tower_lsp::lsp_types::Diagnostic as LspDiagnostic;

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_diagnostic_tags() {
        let tags = infer_diagnostic_tags("unused variable 'x'");
        assert!(tags.contains(&DiagnosticTag::UNNECESSARY));

        let tags = infer_diagnostic_tags("deprecated function");
        assert!(tags.contains(&DiagnosticTag::DEPRECATED));
    }

    #[test]
    fn test_ranges_overlap() {
        let a = Range {
            start: Position { line: 0, character: 0 },
            end: Position { line: 0, character: 10 },
        };
        let b = Range {
            start: Position { line: 0, character: 5 },
            end: Position { line: 0, character: 15 },
        };
        assert!(ranges_overlap(&a, &b));

        let c = Range {
            start: Position { line: 1, character: 0 },
            end: Position { line: 1, character: 10 },
        };
        assert!(!ranges_overlap(&a, &c));
    }

    #[test]
    fn test_incremental_diagnostics_provider() {
        let provider = IncrementalDiagnosticsProvider::new();
        assert!(provider.previous_diagnostics.is_empty());
    }

    #[test]
    fn test_extract_symbol_name() {
        assert_eq!(
            extract_symbol_name("symbol 'Foo' not found"),
            Some("Foo".to_string())
        );
        assert_eq!(
            extract_symbol_name("`Bar` is undefined"),
            Some("Bar".to_string())
        );
        assert_eq!(extract_symbol_name("some other message"), None);
    }
}
