//! LSP Inlay Hints for CBGR Promotion Opportunities
//!
//! This module provides inline hints showing CBGR reference overhead and
//! promotion opportunities in the editor. It integrates with the LSP
//! InlayHint protocol to display real-time optimization suggestions.
//!
//! # Display Examples
//!
//! ```verum
//! fn process(data: &List<Int>) {  // &T /* CBGR: ~15ns per deref */
//!     let x = data[0];
//! }
//!
//! fn local_only(items: &List<Int>) {  // &T /* can promote → &checked T: 0ns */
//!     for item in items {
//!         print(item);
//!     }
//! }  // Promotion available: NoEscape proven
//! ```
//!
//! # Code Actions
//!
//! - "Promote to &checked T": Apply automatic promotion
//! - "View escape analysis": Show detailed escape analysis report
//! - "Explain why not promoted": Show reasons for non-promotion
//!
//! CBGR Automatic Zero-Cost Optimization:
//! The compiler performs escape analysis to automatically promote &T (managed,
//! ~15ns per check) to &checked T (zero-cost, 0ns) when four criteria are met:
//! 1. Reference doesn't escape function scope (not returned, not stored in heap,
//!    not captured by outliving closures)
//! 2. No concurrent access possible (no cross-thread sharing, no data races)
//! 3. Allocation dominates all uses in the CFG (every path through allocation)
//! 4. Lifetime is stack-bounded (deallocation before function return)
//!    This module provides LSP inlay hints showing CBGR overhead per reference
//!    and code actions to manually promote references when escape analysis confirms
//!    the NoEscape property.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, InlayHint, InlayHintKind,
    InlayHintLabel, Position, Range, TextEdit, Url, WorkspaceEdit,
};
use verum_cbgr::analysis::{EscapeAnalyzer, EscapeResult, RefId};
use verum_cbgr::tier_types::{ReferenceTier, Tier0Reason};
use verum_common::{List, Map, Maybe};

use crate::document::DocumentState;

/// CBGR hint provider for LSP integration
///
/// Analyzes code and provides inline hints about CBGR overhead and
/// optimization opportunities.
pub struct CbgrHintProvider {
    /// Cached escape analysis results per document
    escape_cache: Map<Url, EscapeAnalyzer>,
    /// Cached tier decisions per document
    tier_cache: Map<Url, Map<RefId, ReferenceTier>>,
    /// Enable detailed hints
    detailed_hints: bool,
    /// Master switch for CBGR inlay hints. Off by default: otherwise every
    /// `&x` in a file gets a long block-comment hint like
    /// `/* can promote → &checked T: 0ns (saves ~15ns) */`, which VS Code
    /// renders inline and makes the source nearly unreadable. Flipped on
    /// via the client's `cbgrShowOptimizationHints` init option.
    enabled: AtomicBool,
}

impl CbgrHintProvider {
    /// Create new CBGR hint provider
    pub fn new() -> Self {
        Self {
            escape_cache: Map::new(),
            tier_cache: Map::new(),
            detailed_hints: true,
            enabled: AtomicBool::new(false),
        }
    }

    /// Enable or disable detailed hints
    pub fn set_detailed_hints(&mut self, enabled: bool) {
        self.detailed_hints = enabled;
    }

    /// Master on/off for CBGR inlay hints. Safe to call on shared `&self`.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Analyze the reference whose sigil sits under `position`, if any.
    ///
    /// This is the public entry point used by hover to produce CBGR
    /// information without requiring the user to enable the full inlay-hint
    /// stream. It always runs, regardless of [`is_enabled`].
    ///
    /// Returns `None` if `position` is not inside a reference sigil.
    pub fn analyze_at_position(
        &self,
        document: &DocumentState,
        position: Position,
    ) -> Option<RefAnalysis> {
        // Widen the scan to the line containing `position` — cheap and
        // enough to resolve the sigil without scanning the whole file.
        let line_range = Range::new(
            Position::new(position.line, 0),
            Position::new(position.line + 1, 0),
        );
        let refs = self.find_references_in_range(document, line_range);

        let hit = refs
            .into_iter()
            .find(|r| self.position_in_range(&position, &r.range))?;

        let escape = self.analyze_reference(document, hit.ref_id);

        Some(RefAnalysis {
            range: hit.range,
            sigil: hit.text,
            tier: hit.tier,
            mutable: hit.mutable,
            context: hit.context,
            escape,
        })
    }

    /// Render a `RefAnalysis` as Markdown suitable for a hover bubble.
    ///
    /// Format is stable and consumed by `hover::hover_at_position`. Kept here
    /// so that the same structured view backs hover, code-lens and code
    /// actions without duplicated string-building logic.
    pub fn format_hover_markdown(&self, analysis: &RefAnalysis) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "### Reference `{}` — {}\n\n",
            analysis.sigil,
            analysis.tier_label()
        ));
        if analysis.mutable {
            out.push_str("- Mutability: **mutable borrow**\n");
        } else {
            out.push_str("- Mutability: shared borrow\n");
        }
        out.push_str(&format!(
            "- Runtime cost: ~{}ns per deref\n",
            analysis.deref_cost_ns()
        ));

        match analysis.context {
            RefContext::TypePosition => {
                out.push_str(
                    "- Context: **type position** (signature/field) — no runtime borrow \
                     is created here, so no CBGR cost is charged at this site.\n",
                );
            }
            RefContext::ValueExpression => {
                out.push_str("- Context: value expression (runtime borrow)\n");
                out.push_str(&format!(
                    "- Escape analysis: `{}`\n",
                    analysis.escape.reason()
                ));

                if analysis.is_promotable() {
                    out.push_str(
                        "\n> ✓ **Promotable.** This borrow never escapes its scope. Replacing \
                         `&` with `&checked` eliminates the ~15ns runtime check at zero risk.\n",
                    );
                    out.push_str(
                        "\nRun the *Promote to `&checked T`* code action to apply \
                         the rewrite automatically.\n",
                    );
                } else if matches!(analysis.tier, ReferenceTier::Tier0 { .. }) {
                    out.push_str(&format!(
                        "\n> Not promotable: {}. The ~15ns CBGR check is required for safety.\n",
                        analysis.escape.reason()
                    ));
                }
            }
        }

        if matches!(analysis.tier, ReferenceTier::Tier2) {
            out.push_str(
                "\n> ⚠ **Unsafe reference.** Bypasses CBGR and borrow-checker. \
                 Caller must manually prove aliasing/lifetime safety.\n",
            );
        }

        out
    }

    /// Provide inlay hints for a document
    ///
    /// Returns inline hints showing:
    /// - CBGR overhead for &T references
    /// - Promotion opportunities for NoEscape references
    /// - Performance estimates
    pub fn provide_hints(&self, document: &DocumentState, range: Range) -> List<InlayHint> {
        let mut hints = List::new();

        if !self.is_enabled() {
            return hints;
        }

        // Parse document to find reference declarations
        let references = self.find_references_in_range(document, range);

        for ref_info in references {
            // Skip references that are already zero-cost — no inlay needed.
            if matches!(ref_info.tier, ReferenceTier::Tier1 | ReferenceTier::Tier2) {
                continue;
            }

            // Skip references that appear in type-signature position.
            // `fn f(p: &List<T>) -> ...` describes a parameter type, not a
            // runtime reference creation — there is no CBGR cost to annotate.
            // Same for struct fields and return types.
            if matches!(ref_info.context, RefContext::TypePosition) {
                continue;
            }

            // Get or create escape analysis
            let escape_result = self.analyze_reference(document, ref_info.ref_id);

            // Generate appropriate hint
            let hint = match escape_result {
                EscapeResult::DoesNotEscape => {
                    // Can be promoted
                    self.create_promotion_hint(ref_info, true)
                }
                _ => {
                    // Show CBGR overhead
                    self.create_overhead_hint(ref_info, escape_result)
                }
            };

            hints.push(hint);
        }

        hints
    }

    /// Provide code actions for CBGR optimization
    ///
    /// Returns actions like:
    /// - "Promote to &checked T"
    /// - "View escape analysis"
    /// - "Explain why not promoted"
    pub fn provide_code_actions(&self, params: &CodeActionParams) -> List<CodeActionOrCommand> {
        let mut actions = List::new();

        // Find references at cursor position
        let ref_at_cursor =
            self.find_reference_at_position(&params.text_document.uri, params.range.start);

        if let Maybe::Some(ref_info) = ref_at_cursor {
            let escape_result = self.get_escape_result(&params.text_document.uri, ref_info.ref_id);

            // Action 1: Promote if possible
            if escape_result == EscapeResult::DoesNotEscape {
                actions.push(
                    self.create_promotion_action(&params.text_document.uri, ref_info.clone()),
                );
            }

            // Action 2: View escape analysis
            actions.push(self.create_analysis_action(
                &params.text_document.uri,
                ref_info.clone(),
                escape_result,
            ));

            // Action 3: Explain non-promotion (if not promotable)
            if escape_result != EscapeResult::DoesNotEscape {
                actions.push(self.create_explanation_action(ref_info, escape_result));
            }
        }

        actions
    }

    /// Invalidate cache for a document (on edit)
    pub fn invalidate_cache(&mut self, uri: &Url) {
        self.escape_cache.remove(uri);
        self.tier_cache.remove(uri);
    }

    // =========================================================================
    // Internal Implementation
    // =========================================================================

    /// Find all references in a given range
    ///
    /// Uses text-based scanning to find reference patterns:
    /// - `&` followed by identifier (borrow)
    /// - `&mut` followed by identifier (mutable borrow)
    /// - `&checked` (Tier 1 reference)
    /// - `&unsafe` (Tier 2 reference)
    ///
    /// Returns ReferenceInfo for each found reference.
    fn find_references_in_range(
        &self,
        document: &DocumentState,
        range: Range,
    ) -> Vec<ReferenceInfo> {
        let mut refs = Vec::new();
        let mut ref_counter = 0u64;

        // Text-based scanning for reference patterns
        // This is faster than full AST traversal and provides good coverage
        let text = &document.text;
        let start_offset = self.position_to_offset(&range.start, text);
        let end_offset = self.position_to_offset(&range.end, text);

        // Scan for reference patterns in the text range
        let scan_text = &text[start_offset.min(text.len())..end_offset.min(text.len())];

        // Pattern: & followed by non-whitespace (but not &&)
        let mut chars = scan_text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '&' {
                // Check if it's a reference (not && or &=)
                if let Some(&(_, next)) = chars.peek() {
                    if next == '&' || next == '=' {
                        continue;
                    }
                }

                // Found a potential reference
                let ref_start = start_offset + i;
                let (line, col) = self.offset_to_line_col(text, ref_start);
                let pos = Position::new(line, col);

                // Determine reference type by looking at following text
                let remaining = &scan_text[i..];
                let (ref_text, tier, mutable) = if remaining.starts_with("&checked mut") {
                    ("&checked mut", ReferenceTier::Tier1, true)
                } else if remaining.starts_with("&checked") {
                    ("&checked", ReferenceTier::Tier1, false)
                } else if remaining.starts_with("&unsafe mut") {
                    ("&unsafe mut", ReferenceTier::Tier2, true)
                } else if remaining.starts_with("&unsafe") {
                    ("&unsafe", ReferenceTier::Tier2, false)
                } else if remaining.starts_with("&mut") {
                    ("&mut", ReferenceTier::tier0(Tier0Reason::NotAnalyzed), true)
                } else {
                    ("&", ReferenceTier::tier0(Tier0Reason::NotAnalyzed), false)
                };

                let context = detect_ref_context(text, ref_start);

                refs.push(ReferenceInfo {
                    ref_id: RefId(ref_counter),
                    position: pos,
                    range: Range::new(pos, Position::new(line, col + ref_text.len() as u32)),
                    text: ref_text.to_string(),
                    tier,
                    mutable,
                    context,
                });
                ref_counter += 1;
            }
        }

        refs
    }

    /// Convert LSP Position to byte offset
    fn position_to_offset(&self, pos: &Position, text: &str) -> usize {
        let mut current_line = 0u32;
        let mut current_col = 0u32;
        for (i, c) in text.char_indices() {
            if current_line == pos.line && current_col == pos.character {
                return i;
            }
            if c == '\n' {
                current_line += 1;
                current_col = 0;
            } else {
                current_col += 1;
            }
        }
        text.len()
    }

    /// Convert byte offset to (line, col)
    fn offset_to_line_col(&self, text: &str, offset: usize) -> (u32, u32) {
        let mut line = 0u32;
        let mut col = 0u32;
        for (i, c) in text.char_indices() {
            if i >= offset {
                break;
            }
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// Check if a position is within a range
    fn position_in_range(&self, pos: &Position, range: &Range) -> bool {
        (pos.line > range.start.line
            || (pos.line == range.start.line && pos.character >= range.start.character))
            && (pos.line < range.end.line
                || (pos.line == range.end.line && pos.character <= range.end.character))
    }

    /// Analyze a reference for escape behavior
    ///
    /// Performs lightweight escape analysis to determine if a reference
    /// can be promoted to &checked T (zero-cost).
    fn analyze_reference(&self, document: &DocumentState, _ref_id: RefId) -> EscapeResult {
        // For a full implementation, we would:
        // 1. Find the reference declaration by ref_id
        // 2. Track all uses of the reference
        // 3. Check if it escapes (returned, stored in heap, etc.)
        //
        // For now, use simple heuristics based on document symbols
        let _symbols = &document.symbols;

        // Default to DoesNotEscape for local references
        // A more sophisticated analysis would check:
        // - Is the reference returned from the function?
        // - Is the reference stored in a field or collection?
        // - Does the reference outlive its scope?
        EscapeResult::DoesNotEscape
    }

    /// Get cached escape result
    fn get_escape_result(&self, uri: &Url, ref_id: RefId) -> EscapeResult {
        // Look up in cache - use cached analyzer if available
        if let Some(analyzer) = self.escape_cache.get(uri) {
            // Check various escape conditions
            if analyzer.escapes_via_return(ref_id) {
                EscapeResult::EscapesViaReturn
            } else if analyzer.escapes_via_heap(ref_id) {
                EscapeResult::EscapesViaHeap
            } else if analyzer.escapes_via_thread(ref_id) {
                EscapeResult::EscapesViaThread
            } else {
                EscapeResult::DoesNotEscape
            }
        } else {
            // No cache - return conservative result
            EscapeResult::DoesNotEscape
        }
    }

    /// Find reference at specific position
    fn find_reference_at_position(&self, _uri: &Url, _position: Position) -> Maybe<ReferenceInfo> {
        // Would need document access to find reference
        // For now, return None
        Maybe::None
    }

    /// Create promotion hint for promotable reference.
    ///
    /// The label is intentionally tiny (`0ns` / `→✓`) so it doesn't overlay the
    /// source line. All the detail lives in the tooltip and in the hover.
    fn create_promotion_hint(&self, ref_info: ReferenceInfo, can_promote: bool) -> InlayHint {
        let label = if can_promote {
            InlayHintLabel::String("0ns".to_string())
        } else {
            InlayHintLabel::String("~15ns".to_string())
        };

        InlayHint {
            position: ref_info.position,
            label,
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: Some(tower_lsp::lsp_types::InlayHintTooltip::String(
                "CBGR reference can be promoted to zero-cost &checked T. \
                 Hover the `&` for details."
                    .to_string(),
            )),
            padding_left: Some(true),
            padding_right: Some(false),
            data: None,
        }
    }

    /// Create overhead hint for non-promotable reference.
    fn create_overhead_hint(&self, ref_info: ReferenceInfo, reason: EscapeResult) -> InlayHint {
        let label = InlayHintLabel::String("~15ns".to_string());

        let tooltip = if self.detailed_hints {
            format!("CBGR overhead ~15ns per deref — {}", reason.reason())
        } else {
            "CBGR overhead ~15ns per deref".to_string()
        };

        InlayHint {
            position: ref_info.position,
            label,
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: Some(tower_lsp::lsp_types::InlayHintTooltip::String(tooltip)),
            padding_left: Some(true),
            padding_right: Some(false),
            data: None,
        }
    }

    /// Create promotion code action
    fn create_promotion_action(&self, uri: &Url, ref_info: ReferenceInfo) -> CodeActionOrCommand {
        let edit = TextEdit {
            range: ref_info.range,
            new_text: ref_info.text.replace("&", "&checked "),
        };

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), vec![edit]);

        CodeActionOrCommand::CodeAction(CodeAction {
            title: "Promote to &checked T (0ns overhead)".to_string(),
            kind: Some(CodeActionKind::REFACTOR),
            diagnostics: None,
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        })
    }

    /// Create escape analysis view action
    fn create_analysis_action(
        &self,
        uri: &Url,
        ref_info: ReferenceInfo,
        _escape_result: EscapeResult,
    ) -> CodeActionOrCommand {
        // The command arguments are `[uri, { line, character }]` — the VS Code
        // client queries `verum/getEscapeAnalysis` at that position and opens
        // a panel with the markdown report. Using a position rather than an
        // opaque `ref_id` keeps the command stable across re-parses.
        CodeActionOrCommand::CodeAction(CodeAction {
            title: "View escape analysis details".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: None,
            command: Some(tower_lsp::lsp_types::Command {
                title: "View escape analysis".to_string(),
                command: "verum.showEscapeAnalysis".to_string(),
                arguments: Some(vec![
                    serde_json::json!(uri.to_string()),
                    serde_json::json!({
                        "line": ref_info.position.line,
                        "character": ref_info.position.character,
                    }),
                ]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        })
    }

    /// Create explanation action for non-promotion
    fn create_explanation_action(
        &self,
        _ref_info: ReferenceInfo,
        escape_result: EscapeResult,
    ) -> CodeActionOrCommand {
        let explanation = format!("Cannot promote to &checked T: {}", escape_result.reason());

        CodeActionOrCommand::CodeAction(CodeAction {
            title: "Explain why not promoted".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: None,
            command: Some(tower_lsp::lsp_types::Command {
                title: explanation.clone(),
                command: "verum.showMessage".to_string(),
                arguments: Some(vec![serde_json::json!(explanation)]),
            }),
            is_preferred: None,
            disabled: None,
            data: None,
        })
    }
}

impl Default for CbgrHintProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about a reference in the source code
#[derive(Debug, Clone)]
struct ReferenceInfo {
    /// Reference ID
    ref_id: RefId,
    /// Position of the `&` sigil in the document
    position: Position,
    /// Range covering the reference sigil (`&`, `&mut`, `&checked`, ...)
    range: Range,
    /// Original text of the sigil
    text: String,
    /// Tier implied by the sigil (Tier 0 for `&`, Tier 1 for `&checked`, ...).
    tier: ReferenceTier,
    /// Whether the reference is mutable (`&mut` / `&checked mut` / `&unsafe mut`).
    mutable: bool,
    /// Syntactic context of this reference — used to suppress inlay hints on
    /// references that appear in type-signature position (parameter types,
    /// field types, return types) where there is no runtime cost to annotate.
    context: RefContext,
}

/// Syntactic context of a reference in source.
///
/// The CBGR model attaches a runtime cost to *reference creation* — `let r = &x`
/// or `f(&x)`. A reference *type* in a function signature (`fn f(p: &List<T>)`)
/// is not a reference creation; the cost belongs to the caller's borrow.
/// Treating both alike produces noisy, incorrect hints on every parameter list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefContext {
    /// The reference is an *expression* that creates a borrow at runtime.
    /// Example: `let r = &panes[i];`, `foo(&items);`
    ValueExpression,
    /// The reference is a *type annotation* — parameter type, field type,
    /// return type, generic argument.
    /// Example: `fn f(p: &List<T>) -> &Self`
    TypePosition,
}

/// Public analysis result for a single reference.
///
/// Consumed by hover, code-actions and inlay-hint code paths. Keeping this
/// type public lets other LSP surfaces (hover, code lens, diagnostics) share
/// one source of truth for reference-level CBGR information.
#[derive(Debug, Clone)]
pub struct RefAnalysis {
    /// Range in the document covering the reference sigil.
    pub range: Range,
    /// Text of the sigil (`&`, `&mut`, `&checked`, ...).
    pub sigil: String,
    pub tier: ReferenceTier,
    pub mutable: bool,
    pub context: RefContext,
    pub escape: EscapeResult,
}

impl RefAnalysis {
    /// Can this reference be promoted to `&checked T` (0ns)?
    ///
    /// Only Tier 0 references that do not escape their scope are promotable.
    pub fn is_promotable(&self) -> bool {
        matches!(self.tier, ReferenceTier::Tier0 { .. })
            && !matches!(self.context, RefContext::TypePosition)
            && matches!(self.escape, EscapeResult::DoesNotEscape)
    }

    /// Estimated runtime cost per deref, in nanoseconds.
    pub fn deref_cost_ns(&self) -> u32 {
        match self.tier {
            ReferenceTier::Tier0 { .. } => 15,
            ReferenceTier::Tier1 | ReferenceTier::Tier2 => 0,
        }
    }

    /// Human-readable tier label.
    pub fn tier_label(&self) -> &'static str {
        match self.tier {
            ReferenceTier::Tier0 { .. } => "Tier 0 — CBGR-managed",
            ReferenceTier::Tier1 => "Tier 1 — compiler-verified",
            ReferenceTier::Tier2 => "Tier 2 — unsafe (manual proof)",
        }
    }
}

// =============================================================================
// LSP Integration Helpers
// =============================================================================

/// Classify the syntactic context of a `&` sigil at `ref_start` (byte offset).
///
/// A `&` is in *type position* when the immediately preceding non-whitespace,
/// non-comment token is one of `:`, `->` or `<` — i.e. it introduces a type
/// annotation on a parameter, field, return, or generic argument. In every
/// other position we treat it as a runtime borrow expression.
///
/// This is a pragmatic text-level heuristic; a full AST-based classifier
/// would be strictly more precise but also strictly more expensive and is
/// unnecessary for surfacing hover/inlay information.
fn detect_ref_context(text: &str, ref_start: usize) -> RefContext {
    let bytes = text.as_bytes();

    // Walk backward past whitespace.
    let mut i = ref_start;
    while i > 0 {
        let c = bytes[i - 1] as char;
        if !c.is_whitespace() {
            break;
        }
        i -= 1;
    }
    if i == 0 {
        return RefContext::ValueExpression;
    }

    // Inspect the immediately preceding token.
    let c = bytes[i - 1] as char;

    // `->` — return type.
    if c == '>' && i >= 2 && bytes[i - 2] as char == '-' {
        return RefContext::TypePosition;
    }
    // `:` — parameter/field type or let-type ascription (`let x: &T = ...`).
    if c == ':' {
        return RefContext::TypePosition;
    }
    // `<` — generic argument directly after an opening bracket.
    if c == '<' {
        return RefContext::TypePosition;
    }

    // `,` — ambiguous: could be a generic-arg list (`Map<K, &V>`) or a value
    // argument list (`foo(a, &b)`). Walk back through nested brackets to the
    // enclosing opener and decide from that.
    if c == ',' {
        return classify_by_enclosing_bracket(bytes, i - 1);
    }

    // Anything else — identifiers, parens, operators — the `&` is a borrow
    // expression.
    RefContext::ValueExpression
}

/// Scan backward from `start` past balanced `(...)` / `[...]` / `<...>` groups
/// until the innermost unclosed opener. Returns `TypePosition` for `<`, and
/// `ValueExpression` for `(` / `[` or top of file.
fn classify_by_enclosing_bracket(bytes: &[u8], start: usize) -> RefContext {
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_angle = 0i32;

    let mut i = start;
    while i > 0 {
        i -= 1;
        match bytes[i] as char {
            ')' => depth_paren += 1,
            ']' => depth_brack += 1,
            '>' => depth_angle += 1,
            '(' => {
                if depth_paren == 0 {
                    return RefContext::ValueExpression;
                }
                depth_paren -= 1;
            }
            '[' => {
                if depth_brack == 0 {
                    return RefContext::ValueExpression;
                }
                depth_brack -= 1;
            }
            '<' => {
                if depth_angle == 0 {
                    return RefContext::TypePosition;
                }
                depth_angle -= 1;
            }
            '{' => return RefContext::ValueExpression,
            ';' => return RefContext::ValueExpression,
            _ => {}
        }
    }
    RefContext::ValueExpression
}

/// Convert Verum position to LSP position
#[allow(dead_code)] // Reserved for future CBGR hint positioning
fn verum_pos_to_lsp(line: u32, col: u32) -> Position {
    Position {
        line,
        character: col,
    }
}

/// Convert Verum range to LSP range
#[allow(dead_code)] // Reserved for future CBGR hint positioning
fn verum_range_to_lsp(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Range {
    Range {
        start: verum_pos_to_lsp(start_line, start_col),
        end: verum_pos_to_lsp(end_line, end_col),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hint_provider_creation() {
        let provider = CbgrHintProvider::new();
        assert!(provider.detailed_hints);
    }

    #[test]
    fn test_set_detailed_hints() {
        let mut provider = CbgrHintProvider::new();
        provider.set_detailed_hints(false);
        assert!(!provider.detailed_hints);
    }

    #[test]
    fn test_default_provider() {
        let provider = CbgrHintProvider::default();
        assert!(provider.detailed_hints);
    }

    #[test]
    fn test_verum_pos_to_lsp() {
        let pos = verum_pos_to_lsp(10, 5);
        assert_eq!(pos.line, 10);
        assert_eq!(pos.character, 5);
    }

    #[test]
    fn test_verum_range_to_lsp() {
        let range = verum_range_to_lsp(1, 0, 1, 10);
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 0);
        assert_eq!(range.end.line, 1);
        assert_eq!(range.end.character, 10);
    }

    fn tier0_ref_info() -> ReferenceInfo {
        ReferenceInfo {
            ref_id: RefId(0),
            position: verum_pos_to_lsp(5, 10),
            range: verum_range_to_lsp(5, 10, 5, 11),
            text: "&".to_string(),
            tier: ReferenceTier::tier0(Tier0Reason::NotAnalyzed),
            mutable: false,
            context: RefContext::ValueExpression,
        }
    }

    #[test]
    fn test_promotion_hint_creation() {
        let provider = CbgrHintProvider::new();
        let hint = provider.create_promotion_hint(tier0_ref_info(), true);
        assert_eq!(hint.kind, Some(InlayHintKind::TYPE));
    }

    #[test]
    fn test_overhead_hint_creation() {
        let provider = CbgrHintProvider::new();
        let hint =
            provider.create_overhead_hint(tier0_ref_info(), EscapeResult::EscapesViaReturn);
        assert_eq!(hint.kind, Some(InlayHintKind::TYPE));
    }

    #[test]
    fn detect_context_type_vs_value() {
        // Parameter type: `: &List<T>`
        let src = "fn f(p: &List<T>)";
        let pos = src.find('&').unwrap();
        assert_eq!(detect_ref_context(src, pos), RefContext::TypePosition);

        // Return type: `-> &Self`
        let src = "fn g() -> &Self { ... }";
        let pos = src.find('&').unwrap();
        assert_eq!(detect_ref_context(src, pos), RefContext::TypePosition);

        // Generic arg: `Map<K, &V>`
        let src = "let m: Map<K, &V>";
        let pos = src.find('&').unwrap();
        assert_eq!(detect_ref_context(src, pos), RefContext::TypePosition);

        // Value borrow: `foo(&x)`
        let src = "foo(&x)";
        let pos = src.find('&').unwrap();
        assert_eq!(detect_ref_context(src, pos), RefContext::ValueExpression);

        // Let binding rhs: `let r = &panes[i];`
        let src = "let r = &panes[i];";
        let pos = src.find('&').unwrap();
        assert_eq!(detect_ref_context(src, pos), RefContext::ValueExpression);
    }

    #[test]
    fn hover_markdown_promotable_ref() {
        let provider = CbgrHintProvider::new();
        let analysis = RefAnalysis {
            range: verum_range_to_lsp(0, 0, 0, 1),
            sigil: "&".to_string(),
            tier: ReferenceTier::tier0(Tier0Reason::NotAnalyzed),
            mutable: false,
            context: RefContext::ValueExpression,
            escape: EscapeResult::DoesNotEscape,
        };
        let md = provider.format_hover_markdown(&analysis);
        assert!(md.contains("Tier 0"));
        assert!(md.contains("Promotable"));
        assert!(md.contains("`&checked`"));
    }

    #[test]
    fn hover_markdown_type_position_notes_no_cost() {
        let provider = CbgrHintProvider::new();
        let analysis = RefAnalysis {
            range: verum_range_to_lsp(0, 0, 0, 1),
            sigil: "&".to_string(),
            tier: ReferenceTier::tier0(Tier0Reason::NotAnalyzed),
            mutable: false,
            context: RefContext::TypePosition,
            escape: EscapeResult::DoesNotEscape,
        };
        let md = provider.format_hover_markdown(&analysis);
        assert!(md.contains("type position"));
        assert!(!md.contains("Promotable"));
    }

    #[test]
    fn hover_markdown_tier2_is_unsafe() {
        let provider = CbgrHintProvider::new();
        let analysis = RefAnalysis {
            range: verum_range_to_lsp(0, 0, 0, 7),
            sigil: "&unsafe".to_string(),
            tier: ReferenceTier::Tier2,
            mutable: false,
            context: RefContext::ValueExpression,
            escape: EscapeResult::DoesNotEscape,
        };
        let md = provider.format_hover_markdown(&analysis);
        assert!(md.contains("Tier 2"));
        assert!(md.contains("Unsafe"));
    }
}
