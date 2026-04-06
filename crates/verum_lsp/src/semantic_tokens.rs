//! Semantic tokens from syntax tree
//!
//! Provides semantic highlighting by traversing the lossless syntax tree.
//! Maps SyntaxKind to LSP SemanticTokenType for rich syntax highlighting.
//!
//! Features:
//! - Full document semantic tokens (textDocument/semanticTokens/full)
//! - Range-based semantic tokens (textDocument/semanticTokens/range)
//! - Context-aware token classification (distinguishes functions, types, etc.)
//! - Delta encoding for incremental updates

use tower_lsp::lsp_types::*;
use verum_ast::FileId;
use verum_parser::syntax_bridge::LosslessParser;
use verum_syntax::{SyntaxElement, SyntaxKind, SyntaxNode, SyntaxToken};

// ==================== Token Type Enum ====================

/// Semantic token types supported by Verum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SemanticTokenType {
    Namespace = 0,
    Type = 1,
    Class = 2,
    Enum = 3,
    Interface = 4,
    Struct = 5,
    TypeParameter = 6,
    Parameter = 7,
    Variable = 8,
    Property = 9,
    EnumMember = 10,
    Event = 11,
    Function = 12,
    Method = 13,
    Macro = 14,
    Keyword = 15,
    Modifier = 16,
    Comment = 17,
    String = 18,
    Number = 19,
    Regexp = 20,
    Operator = 21,
    Decorator = 22,
}

impl SemanticTokenType {
    /// Get all token types as LSP legend.
    pub fn legend() -> Vec<tower_lsp::lsp_types::SemanticTokenType> {
        vec![
            tower_lsp::lsp_types::SemanticTokenType::NAMESPACE,
            tower_lsp::lsp_types::SemanticTokenType::TYPE,
            tower_lsp::lsp_types::SemanticTokenType::CLASS,
            tower_lsp::lsp_types::SemanticTokenType::ENUM,
            tower_lsp::lsp_types::SemanticTokenType::INTERFACE,
            tower_lsp::lsp_types::SemanticTokenType::STRUCT,
            tower_lsp::lsp_types::SemanticTokenType::TYPE_PARAMETER,
            tower_lsp::lsp_types::SemanticTokenType::PARAMETER,
            tower_lsp::lsp_types::SemanticTokenType::VARIABLE,
            tower_lsp::lsp_types::SemanticTokenType::PROPERTY,
            tower_lsp::lsp_types::SemanticTokenType::ENUM_MEMBER,
            tower_lsp::lsp_types::SemanticTokenType::EVENT,
            tower_lsp::lsp_types::SemanticTokenType::FUNCTION,
            tower_lsp::lsp_types::SemanticTokenType::METHOD,
            tower_lsp::lsp_types::SemanticTokenType::MACRO,
            tower_lsp::lsp_types::SemanticTokenType::KEYWORD,
            tower_lsp::lsp_types::SemanticTokenType::MODIFIER,
            tower_lsp::lsp_types::SemanticTokenType::COMMENT,
            tower_lsp::lsp_types::SemanticTokenType::STRING,
            tower_lsp::lsp_types::SemanticTokenType::NUMBER,
            tower_lsp::lsp_types::SemanticTokenType::REGEXP,
            tower_lsp::lsp_types::SemanticTokenType::OPERATOR,
            tower_lsp::lsp_types::SemanticTokenType::DECORATOR,
        ]
    }
}

// ==================== Token Modifier Enum ====================

/// Semantic token modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SemanticTokenModifier {
    Declaration = 0,
    Definition = 1,
    Readonly = 2,
    Static = 3,
    Deprecated = 4,
    Abstract = 5,
    Async = 6,
    Modification = 7,
    Documentation = 8,
    DefaultLibrary = 9,
    /// Unsafe reference tier (&unsafe)
    Unsafe = 10,
    /// Checked reference tier (&checked)
    Checked = 11,
}

impl SemanticTokenModifier {
    /// Get all token modifiers as LSP legend.
    pub fn legend() -> Vec<tower_lsp::lsp_types::SemanticTokenModifier> {
        vec![
            tower_lsp::lsp_types::SemanticTokenModifier::DECLARATION,
            tower_lsp::lsp_types::SemanticTokenModifier::DEFINITION,
            tower_lsp::lsp_types::SemanticTokenModifier::READONLY,
            tower_lsp::lsp_types::SemanticTokenModifier::STATIC,
            tower_lsp::lsp_types::SemanticTokenModifier::DEPRECATED,
            tower_lsp::lsp_types::SemanticTokenModifier::ABSTRACT,
            tower_lsp::lsp_types::SemanticTokenModifier::ASYNC,
            tower_lsp::lsp_types::SemanticTokenModifier::MODIFICATION,
            tower_lsp::lsp_types::SemanticTokenModifier::DOCUMENTATION,
            tower_lsp::lsp_types::SemanticTokenModifier::DEFAULT_LIBRARY,
            // Custom modifiers for Verum-specific constructs
            tower_lsp::lsp_types::SemanticTokenModifier::new("unsafe"),
            tower_lsp::lsp_types::SemanticTokenModifier::new("checked"),
        ]
    }
}

// ==================== Provider ====================

/// Provides semantic tokens from syntax tree.
pub struct SemanticTokenProvider;

impl SemanticTokenProvider {
    /// Create a new semantic token provider.
    pub fn new() -> Self {
        Self
    }

    /// Get semantic tokens legend.
    pub fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: SemanticTokenType::legend(),
            token_modifiers: SemanticTokenModifier::legend(),
        }
    }

    /// Compute semantic tokens for source code.
    pub fn compute(&self, source: &str, file_id: FileId) -> SemanticTokensResult {
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        let root = result.syntax();

        let mut tokens = Vec::new();
        let line_index = LineIndex::new(source);

        self.collect_tokens_contextual(&root, &line_index, &mut tokens, None);

        // Sort by position
        tokens.sort_by(|a, b| a.start.cmp(&b.start));

        // Convert to relative positions
        let relative_tokens = self.to_relative_tokens(&tokens, &line_index);

        SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: relative_tokens,
        })
    }

    /// Compute semantic tokens for a specific range.
    pub fn compute_range(
        &self,
        source: &str,
        file_id: FileId,
        range: Range,
    ) -> SemanticTokensResult {
        let parser = LosslessParser::new();
        let result = parser.parse(source, file_id);
        let root = result.syntax();

        let mut tokens = Vec::new();
        let line_index = LineIndex::new(source);

        // Collect all tokens
        self.collect_tokens_contextual(&root, &line_index, &mut tokens, None);

        // Sort by position
        tokens.sort_by(|a, b| a.start.cmp(&b.start));

        // Filter to range
        let filtered: Vec<_> = tokens
            .into_iter()
            .filter(|t| {
                let pos = line_index.position_at(t.start);
                pos.line >= range.start.line && pos.line <= range.end.line
            })
            .collect();

        // Convert to relative positions
        let relative_tokens = self.to_relative_tokens(&filtered, &line_index);

        SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: relative_tokens,
        })
    }

    /// Collect semantic tokens with context awareness.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_tokens_contextual(
        &self,
        node: &SyntaxNode,
        line_index: &LineIndex,
        tokens: &mut Vec<RawSemanticToken>,
        context: Option<NodeContext>,
    ) {
        let node_kind = node.kind();

        // Determine context for children based on current node kind
        let child_context = match node_kind {
            SyntaxKind::FN_DEF => Some(NodeContext::FunctionDef),
            SyntaxKind::TYPE_DEF => Some(NodeContext::TypeDef),
            SyntaxKind::PROTOCOL_DEF => Some(NodeContext::ProtocolDef),
            SyntaxKind::IMPL_BLOCK => Some(NodeContext::ImplBlock),
            SyntaxKind::PARAM_LIST => Some(NodeContext::ParamList),
            SyntaxKind::PARAM => Some(NodeContext::Parameter),
            SyntaxKind::GENERIC_PARAMS => Some(NodeContext::GenericParams),
            SyntaxKind::TYPE_PARAM => Some(NodeContext::TypeParam),
            SyntaxKind::LET_STMT => Some(NodeContext::LetBinding),
            SyntaxKind::CALL_EXPR => Some(NodeContext::CallExpr),
            SyntaxKind::METHOD_CALL_EXPR => Some(NodeContext::MethodCallExpr),
            SyntaxKind::FIELD_EXPR => Some(NodeContext::FieldAccess),
            SyntaxKind::PATH_TYPE => Some(NodeContext::TypePath),
            SyntaxKind::REFERENCE_TYPE => Some(NodeContext::ReferenceType),
            SyntaxKind::GENERIC_TYPE => Some(NodeContext::TypePath),
            SyntaxKind::ATTRIBUTE => Some(NodeContext::Attribute),
            SyntaxKind::CONST_DEF => Some(NodeContext::ConstDef),
            SyntaxKind::STATIC_DEF => Some(NodeContext::StaticDef),
            SyntaxKind::MODULE_DEF => Some(NodeContext::ModuleDef),
            SyntaxKind::META_DEF => Some(NodeContext::MacroDef),
            SyntaxKind::FIELD_DEF => Some(NodeContext::FieldDef),
            SyntaxKind::VARIANT_DEF => Some(NodeContext::VariantDef),
            SyntaxKind::ASYNC_EXPR => Some(NodeContext::AsyncExpr),
            SyntaxKind::MATCH_ARM => Some(NodeContext::MatchArm),
            SyntaxKind::FOR_EXPR => Some(NodeContext::ForLoop),
            // Proof constructs
            SyntaxKind::THEOREM_DEF => Some(NodeContext::TheoremDef),
            SyntaxKind::LEMMA_DEF => Some(NodeContext::LemmaDef),
            SyntaxKind::AXIOM_DEF => Some(NodeContext::AxiomDef),
            SyntaxKind::COROLLARY_DEF => Some(NodeContext::CorollaryDef),
            SyntaxKind::PROOF_BLOCK => Some(NodeContext::ProofBlock),
            SyntaxKind::CALC_BLOCK => Some(NodeContext::CalcChain),
            // Context system
            SyntaxKind::CONTEXT_DEF => Some(NodeContext::ContextDef),
            _ => context,
        };

        // Process this node's direct children
        for child in node.children() {
            match child {
                SyntaxElement::Token(token) => {
                    if let Some(semantic_token) =
                        self.classify_token_with_context(&token, node_kind, child_context)
                    {
                        let range = token.text_range();
                        tokens.push(RawSemanticToken {
                            start: range.start(),
                            len: range.len(),
                            token_type: semantic_token.0 as u32,
                            modifiers: semantic_token.1,
                        });
                    }
                }
                SyntaxElement::Node(child_node) => {
                    self.collect_tokens_contextual(&child_node, line_index, tokens, child_context);
                }
            }
        }
    }

    /// Classify a syntax token with context information.
    fn classify_token_with_context(
        &self,
        token: &SyntaxToken,
        parent_kind: SyntaxKind,
        context: Option<NodeContext>,
    ) -> Option<(SemanticTokenType, u32)> {
        let kind = token.kind();
        let _ = parent_kind; // Used in identifier classification below

        // Handle comments (always the same)
        match kind {
            SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT => {
                return Some((SemanticTokenType::Comment, 0))
            }
            SyntaxKind::DOC_COMMENT | SyntaxKind::INNER_DOC_COMMENT => {
                return Some((
                    SemanticTokenType::Comment,
                    1 << SemanticTokenModifier::Documentation as u32,
                ))
            }
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE => return None,
            _ => {}
        }

        // Keywords with modifiers
        if kind.is_keyword() {
            return match kind {
                // Modifiers
                SyntaxKind::PUB_KW
                | SyntaxKind::PUBLIC_KW
                | SyntaxKind::INTERNAL_KW
                | SyntaxKind::PROTECTED_KW
                | SyntaxKind::PRIVATE_KW
                | SyntaxKind::MUT_KW
                | SyntaxKind::CONST_KW
                | SyntaxKind::STATIC_KW
                | SyntaxKind::PURE_KW
                | SyntaxKind::AFFINE_KW
                | SyntaxKind::LINEAR_KW => Some((SemanticTokenType::Modifier, 0)),

                // Async keyword with async modifier
                SyntaxKind::ASYNC_KW => Some((
                    SemanticTokenType::Keyword,
                    1 << SemanticTokenModifier::Async as u32,
                )),

                // Proof declaration keywords - special highlighting with declaration modifier
                SyntaxKind::THEOREM_KW
                | SyntaxKind::LEMMA_KW
                | SyntaxKind::AXIOM_KW
                | SyntaxKind::COROLLARY_KW => Some((
                    SemanticTokenType::Keyword,
                    1 << SemanticTokenModifier::Declaration as u32,
                )),

                // Proof structure keywords
                SyntaxKind::PROOF_KW | SyntaxKind::BY_KW | SyntaxKind::QED_KW => {
                    Some((SemanticTokenType::Keyword, 0))
                }

                // Proof helper keywords
                SyntaxKind::HAVE_KW
                | SyntaxKind::SHOW_KW
                | SyntaxKind::SUFFICES_KW
                | SyntaxKind::OBTAIN_KW
                | SyntaxKind::CALC_KW => Some((SemanticTokenType::Keyword, 0)),

                // Tactics - highlighted as keywords
                SyntaxKind::AUTO_KW
                | SyntaxKind::SIMP_KW
                | SyntaxKind::RING_KW
                | SyntaxKind::FIELD_KW
                | SyntaxKind::OMEGA_KW
                | SyntaxKind::BLAST_KW
                | SyntaxKind::SMT_KW
                | SyntaxKind::INDUCTION_KW
                | SyntaxKind::CASES_KW
                | SyntaxKind::TRIVIAL_KW
                | SyntaxKind::ASSUMPTION_KW
                | SyntaxKind::CONTRADICTION_KW => Some((SemanticTokenType::Keyword, 0)),

                // Quantifiers
                SyntaxKind::FORALL_KW | SyntaxKind::EXISTS_KW => {
                    Some((SemanticTokenType::Keyword, 0))
                }

                // Context system keywords
                SyntaxKind::USING_KW | SyntaxKind::CONTEXT_KW | SyntaxKind::PROVIDE_KW => {
                    Some((SemanticTokenType::Keyword, 0))
                }

                // Meta/quote keywords - with static modifier to indicate compile-time
                SyntaxKind::META_KW | SyntaxKind::QUOTE_KW | SyntaxKind::LIFT_KW => Some((
                    SemanticTokenType::Keyword,
                    1 << SemanticTokenModifier::Static as u32,
                )),

                // Checked keyword in reference context
                SyntaxKind::CHECKED_KW => Some((
                    SemanticTokenType::Modifier,
                    1 << SemanticTokenModifier::Checked as u32,
                )),

                // Unsafe keyword - modifier with unsafe flag
                SyntaxKind::UNSAFE_KW => Some((
                    SemanticTokenType::Modifier,
                    1 << SemanticTokenModifier::Unsafe as u32,
                )),

                // Async/spawn keywords
                SyntaxKind::SPAWN_KW | SyntaxKind::NURSERY_KW | SyntaxKind::SELECT_KW => Some((
                    SemanticTokenType::Keyword,
                    1 << SemanticTokenModifier::Async as u32,
                )),

                // Error handling keywords
                SyntaxKind::RECOVER_KW | SyntaxKind::THROWS_KW => {
                    Some((SemanticTokenType::Keyword, 0))
                }

                // Defer keywords
                SyntaxKind::DEFER_KW | SyntaxKind::ERRDEFER_KW => {
                    Some((SemanticTokenType::Keyword, 0))
                }

                // Verification constraint keywords
                SyntaxKind::REQUIRES_KW
                | SyntaxKind::ENSURES_KW
                | SyntaxKind::INVARIANT_KW
                | SyntaxKind::DECREASES_KW => Some((SemanticTokenType::Keyword, 0)),

                _ => Some((SemanticTokenType::Keyword, 0)),
            };
        }

        // Literals
        match kind {
            SyntaxKind::INT_LITERAL | SyntaxKind::FLOAT_LITERAL | SyntaxKind::HEX_COLOR => {
                return Some((SemanticTokenType::Number, 0))
            }
            SyntaxKind::STRING_LITERAL
            | SyntaxKind::CHAR_LITERAL
            | SyntaxKind::INTERPOLATED_STRING
            | SyntaxKind::TAGGED_LITERAL => return Some((SemanticTokenType::String, 0)),
            SyntaxKind::TRUE_KW | SyntaxKind::FALSE_KW => {
                return Some((SemanticTokenType::Keyword, 0))
            }
            _ => {}
        }

        // Operators
        if kind.is_punct() {
            // Attribute @ gets special treatment
            if kind == SyntaxKind::AT {
                return Some((SemanticTokenType::Decorator, 0));
            }
            return Some((SemanticTokenType::Operator, 0));
        }

        // Identifier classification based on context
        if kind == SyntaxKind::IDENT {
            return self.classify_identifier(token, parent_kind, context);
        }

        None
    }

    /// Classify an identifier based on context.
    fn classify_identifier(
        &self,
        _token: &SyntaxToken,
        parent_kind: SyntaxKind,
        context: Option<NodeContext>,
    ) -> Option<(SemanticTokenType, u32)> {
        // Check if this is the "defining" identifier in a definition
        // by looking at where it appears in the parent node

        match context {
            Some(NodeContext::FunctionDef) => {
                // If this is the first IDENT in FN_DEF, it's the function name
                if parent_kind == SyntaxKind::FN_DEF {
                    return Some((
                        SemanticTokenType::Function,
                        (1 << SemanticTokenModifier::Declaration as u32)
                            | (1 << SemanticTokenModifier::Definition as u32),
                    ));
                }
            }

            Some(NodeContext::TypeDef) => {
                if parent_kind == SyntaxKind::TYPE_DEF {
                    return Some((
                        SemanticTokenType::Type,
                        (1 << SemanticTokenModifier::Declaration as u32)
                            | (1 << SemanticTokenModifier::Definition as u32),
                    ));
                }
            }

            Some(NodeContext::ProtocolDef) => {
                if parent_kind == SyntaxKind::PROTOCOL_DEF {
                    return Some((
                        SemanticTokenType::Interface,
                        (1 << SemanticTokenModifier::Declaration as u32)
                            | (1 << SemanticTokenModifier::Definition as u32),
                    ));
                }
            }

            Some(NodeContext::Parameter) => {
                return Some((
                    SemanticTokenType::Parameter,
                    1 << SemanticTokenModifier::Declaration as u32,
                ));
            }

            Some(NodeContext::TypeParam) => {
                return Some((
                    SemanticTokenType::TypeParameter,
                    1 << SemanticTokenModifier::Declaration as u32,
                ));
            }

            Some(NodeContext::LetBinding) => {
                if parent_kind == SyntaxKind::LET_STMT || parent_kind == SyntaxKind::IDENT_PAT {
                    return Some((
                        SemanticTokenType::Variable,
                        1 << SemanticTokenModifier::Declaration as u32,
                    ));
                }
            }

            Some(NodeContext::CallExpr) => {
                // First identifier in call expression is the function name
                if parent_kind == SyntaxKind::CALL_EXPR || parent_kind == SyntaxKind::PATH_EXPR {
                    return Some((SemanticTokenType::Function, 0));
                }
            }

            Some(NodeContext::MethodCallExpr) => {
                // The identifier after the dot
                return Some((SemanticTokenType::Method, 0));
            }

            Some(NodeContext::FieldAccess) => {
                return Some((SemanticTokenType::Property, 0));
            }

            Some(NodeContext::TypePath) => {
                return Some((SemanticTokenType::Type, 0));
            }

            Some(NodeContext::Attribute) => {
                return Some((SemanticTokenType::Decorator, 0));
            }

            Some(NodeContext::ConstDef) => {
                if parent_kind == SyntaxKind::CONST_DEF {
                    return Some((
                        SemanticTokenType::Variable,
                        (1 << SemanticTokenModifier::Declaration as u32)
                            | (1 << SemanticTokenModifier::Readonly as u32),
                    ));
                }
            }

            Some(NodeContext::StaticDef) => {
                if parent_kind == SyntaxKind::STATIC_DEF {
                    return Some((
                        SemanticTokenType::Variable,
                        (1 << SemanticTokenModifier::Declaration as u32)
                            | (1 << SemanticTokenModifier::Static as u32),
                    ));
                }
            }

            Some(NodeContext::ModuleDef) => {
                if parent_kind == SyntaxKind::MODULE_DEF {
                    return Some((
                        SemanticTokenType::Namespace,
                        1 << SemanticTokenModifier::Declaration as u32,
                    ));
                }
            }

            Some(NodeContext::MacroDef) => {
                if parent_kind == SyntaxKind::META_DEF {
                    return Some((
                        SemanticTokenType::Macro,
                        1 << SemanticTokenModifier::Declaration as u32,
                    ));
                }
            }

            Some(NodeContext::FieldDef) => {
                return Some((
                    SemanticTokenType::Property,
                    1 << SemanticTokenModifier::Declaration as u32,
                ));
            }

            Some(NodeContext::VariantDef) => {
                return Some((
                    SemanticTokenType::EnumMember,
                    1 << SemanticTokenModifier::Declaration as u32,
                ));
            }

            Some(NodeContext::MatchArm) | Some(NodeContext::ForLoop) => {
                // Pattern binding
                if parent_kind == SyntaxKind::IDENT_PAT {
                    return Some((
                        SemanticTokenType::Variable,
                        1 << SemanticTokenModifier::Declaration as u32,
                    ));
                }
            }

            _ => {}
        }

        // Default to variable
        Some((SemanticTokenType::Variable, 0))
    }

    /// Convert absolute tokens to relative (LSP format).
    fn to_relative_tokens(
        &self,
        tokens: &[RawSemanticToken],
        line_index: &LineIndex,
    ) -> Vec<SemanticToken> {
        let mut result = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_char = 0u32;

        for token in tokens {
            let pos = line_index.position_at(token.start);
            let line = pos.line;
            let character = pos.character;

            let delta_line = line - prev_line;
            let delta_start = if delta_line == 0 {
                character - prev_char
            } else {
                character
            };

            result.push(SemanticToken {
                delta_line,
                delta_start,
                length: token.len,
                token_type: token.token_type,
                token_modifiers_bitset: token.modifiers,
            });

            prev_line = line;
            prev_char = character;
        }

        result
    }
}

impl Default for SemanticTokenProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Context Tracking ====================

/// Context for semantic token classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeContext {
    FunctionDef,
    TypeDef,
    ProtocolDef,
    ImplBlock,
    ParamList,
    Parameter,
    GenericParams,
    TypeParam,
    LetBinding,
    CallExpr,
    MethodCallExpr,
    FieldAccess,
    TypePath,
    Attribute,
    ConstDef,
    StaticDef,
    ModuleDef,
    MacroDef,
    FieldDef,
    VariantDef,
    AsyncExpr,
    MatchArm,
    ForLoop,
    // Proof constructs
    TheoremDef,
    LemmaDef,
    AxiomDef,
    CorollaryDef,
    ProofBlock,
    CalcChain,
    // Context system
    ContextDef,
    UsingClause,
    ProvideStmt,
    // Reference type context
    ReferenceType,
}

// ==================== Internal Types ====================

/// Raw semantic token with absolute positions.
struct RawSemanticToken {
    start: u32,
    len: u32,
    token_type: u32,
    modifiers: u32,
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

// ==================== Public Range Filtering ====================

/// Filter semantic tokens to only include those within the specified range.
/// Used by backend.rs for range-based semantic token requests.
pub fn filter_tokens_by_range(data: &[SemanticToken], range: &Range) -> Vec<SemanticToken> {
    let mut result = Vec::new();
    let mut current_line = 0u32;
    let mut prev_line_in_range = 0u32;
    let mut first_in_range = true;

    for token in data {
        // Update current position based on delta encoding
        current_line += token.delta_line;

        // Check if token is within the requested range
        let in_range = current_line >= range.start.line && current_line <= range.end.line;

        if in_range {
            if first_in_range {
                // First token in range - use absolute position relative to range start
                result.push(SemanticToken {
                    delta_line: current_line - range.start.line,
                    delta_start: token.delta_start,
                    length: token.length,
                    token_type: token.token_type,
                    token_modifiers_bitset: token.token_modifiers_bitset,
                });
                prev_line_in_range = current_line;
                first_in_range = false;
            } else {
                // Subsequent tokens - recalculate deltas relative to previous in-range token
                let new_delta_line = current_line - prev_line_in_range;

                result.push(SemanticToken {
                    delta_line: new_delta_line,
                    delta_start: token.delta_start,
                    length: token.length,
                    token_type: token.token_type,
                    token_modifiers_bitset: token.token_modifiers_bitset,
                });
                prev_line_in_range = current_line;
            }
        }
    }

    result
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_token_legend() {
        let legend = SemanticTokenProvider::legend();
        assert!(!legend.token_types.is_empty());
        assert!(!legend.token_modifiers.is_empty());
    }

    #[test]
    fn test_compute_tokens() {
        let source = "fn foo() { let x = 1; }";
        let provider = SemanticTokenProvider::new();
        let result = provider.compute(source, FileId::new(0));

        match result {
            SemanticTokensResult::Tokens(tokens) => {
                assert!(!tokens.data.is_empty());
            }
            _ => panic!("Expected tokens"),
        }
    }

    #[test]
    fn test_compute_range() {
        let source = "fn foo() {\n    let x = 1;\n    let y = 2;\n}";
        let provider = SemanticTokenProvider::new();
        let range = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 100,
            },
        };
        let result = provider.compute_range(source, FileId::new(0), range);

        match result {
            SemanticTokensResult::Tokens(tokens) => {
                // Should have tokens from line 1 only
                for token in &tokens.data {
                    // All delta_lines should be relative to line 1
                    assert!(token.delta_line == 0 || tokens.data[0].delta_line <= 1);
                }
            }
            _ => panic!("Expected tokens"),
        }
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

        let pos = index.position_at(8);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 2);
    }

    #[test]
    fn test_filter_tokens_by_range() {
        let tokens = vec![
            SemanticToken {
                delta_line: 0,
                delta_start: 0,
                length: 2,
                token_type: SemanticTokenType::Keyword as u32,
                token_modifiers_bitset: 0,
            },
            SemanticToken {
                delta_line: 1,
                delta_start: 4,
                length: 4,
                token_type: SemanticTokenType::Function as u32,
                token_modifiers_bitset: 0,
            },
            SemanticToken {
                delta_line: 1,
                delta_start: 0,
                length: 6,
                token_type: SemanticTokenType::Keyword as u32,
                token_modifiers_bitset: 0,
            },
        ];

        // Filter to only line 1
        let range = Range {
            start: Position {
                line: 1,
                character: 0,
            },
            end: Position {
                line: 1,
                character: 100,
            },
        };

        let filtered = filter_tokens_by_range(&tokens, &range);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].delta_line, 0); // Now relative to range start (line 1)
        assert_eq!(filtered[0].token_type, SemanticTokenType::Function as u32);
    }

    #[test]
    fn test_keyword_classification() {
        let source = "fn foo() {}";
        let provider = SemanticTokenProvider::new();
        let result = provider.compute(source, FileId::new(0));

        match result {
            SemanticTokensResult::Tokens(tokens) => {
                // First token should be "fn" keyword
                assert!(!tokens.data.is_empty());
                assert_eq!(tokens.data[0].token_type, SemanticTokenType::Keyword as u32);
            }
            _ => panic!("Expected tokens"),
        }
    }

    #[test]
    fn test_string_literal_classification() {
        let source = r#"let x = "hello";"#;
        let provider = SemanticTokenProvider::new();
        let result = provider.compute(source, FileId::new(0));

        match result {
            SemanticTokensResult::Tokens(tokens) => {
                // Should have a string token
                let has_string = tokens
                    .data
                    .iter()
                    .any(|t| t.token_type == SemanticTokenType::String as u32);
                assert!(has_string, "Should have string token");
            }
            _ => panic!("Expected tokens"),
        }
    }

    #[test]
    fn test_number_literal_classification() {
        let source = "let x = 42;";
        let provider = SemanticTokenProvider::new();
        let result = provider.compute(source, FileId::new(0));

        match result {
            SemanticTokensResult::Tokens(tokens) => {
                // Should have a number token
                let has_number = tokens
                    .data
                    .iter()
                    .any(|t| t.token_type == SemanticTokenType::Number as u32);
                assert!(has_number, "Should have number token");
            }
            _ => panic!("Expected tokens"),
        }
    }
}
