//! Exhaustiveness Checking Integration for LSP
//!
//! This module provides real-time exhaustiveness feedback in the IDE through
//! integration with the exhaustiveness checking system in verum_types.
//!
//! ## Features
//!
//! - Non-exhaustive match warnings with witness examples
//! - Redundant pattern detection (unreachable patterns)
//! - Quick fixes for adding missing patterns
//! - Hover information for pattern coverage
//!
//! ## Integration
//!
//! The module integrates with:
//! - verum_types::exhaustiveness for the core algorithm
//! - tower_lsp for diagnostic publishing
//! - crate::diagnostics for diagnostic formatting

use crate::diagnostics::LspDiagnostic;
use std::collections::HashMap;
use tower_lsp::lsp_types::*;
use verum_common::{List, Text};

/// Exhaustiveness diagnostic for LSP
#[derive(Debug, Clone)]
pub struct ExhaustivenessDiagnostic {
    /// The range of the match expression
    pub range: Range,
    /// Whether the match is exhaustive
    pub is_exhaustive: bool,
    /// Uncovered cases as string representations
    pub uncovered_cases: List<Text>,
    /// Indices of redundant patterns
    pub redundant_patterns: List<usize>,
    /// Whether all patterns are guarded
    pub all_guarded: bool,
    /// URI of the document
    pub uri: Url,
}

impl ExhaustivenessDiagnostic {
    /// Convert to LSP diagnostics
    pub fn to_lsp_diagnostics(&self) -> Vec<LspDiagnostic> {
        let mut diagnostics = Vec::new();

        // Non-exhaustive error
        if !self.is_exhaustive {
            let message = if self.uncovered_cases.len() == 1 {
                format!(
                    "non-exhaustive patterns: `{}` not covered",
                    self.uncovered_cases.first().map(|s| s.as_str()).unwrap_or("_")
                )
            } else if self.uncovered_cases.len() <= 3 {
                let cases: Vec<_> = self
                    .uncovered_cases
                    .iter()
                    .map(|s| format!("`{}`", s))
                    .collect();
                format!("non-exhaustive patterns: {} not covered", cases.join(", "))
            } else {
                let cases: Vec<_> = self
                    .uncovered_cases
                    .iter()
                    .take(3)
                    .map(|s| format!("`{}`", s))
                    .collect();
                format!(
                    "non-exhaustive patterns: {} and {} other(s) not covered",
                    cases.join(", "),
                    self.uncovered_cases.len() - 3
                )
            };

            diagnostics.push(LspDiagnostic {
                range: self.range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String("E0601".to_string())),
                source: Some("verum".to_string()),
                message,
                related_information: None,
                tags: None,
                code_description: Some(CodeDescription {
                    href: Url::parse("https://verum-lang.org/errors/E0601")
                        .unwrap_or_else(|_| self.uri.clone()),
                }),
                data: None,
            });
        }

        // All guarded warning
        if self.all_guarded && self.is_exhaustive {
            diagnostics.push(LspDiagnostic {
                range: self.range,
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("W0603".to_string())),
                source: Some("verum".to_string()),
                message: "match expression has only guarded patterns; if all guards evaluate to false, no arm will match".to_string(),
                related_information: None,
                tags: None,
                code_description: Some(CodeDescription {
                    href: Url::parse("https://verum-lang.org/errors/W0603")
                        .unwrap_or_else(|_| self.uri.clone()),
                }),
                data: None,
            });
        }

        diagnostics
    }
}

/// Create quick fix for adding missing patterns
pub fn create_add_patterns_fix(
    diagnostic: &ExhaustivenessDiagnostic,
    match_end_range: Range,
) -> CodeAction {
    // Generate pattern arms for uncovered cases
    let arms: Vec<_> = diagnostic
        .uncovered_cases
        .iter()
        .map(|case| format!("        {} => todo!(),", case))
        .collect();

    let new_text = if arms.is_empty() {
        "        _ => todo!(),\n".to_string()
    } else {
        format!("{}\n", arms.join("\n"))
    };

    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = HashMap::new();
            changes.insert(
                diagnostic.uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: match_end_range.start.line,
                            character: 0,
                        },
                        end: Position {
                            line: match_end_range.start.line,
                            character: 0,
                        },
                    },
                    new_text,
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: "Add missing patterns".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(true),
        disabled: None,
        data: None,
    }
}

/// Create quick fix for adding wildcard pattern
pub fn create_add_wildcard_fix(
    diagnostic: &ExhaustivenessDiagnostic,
    match_end_range: Range,
) -> CodeAction {
    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = HashMap::new();
            changes.insert(
                diagnostic.uri.clone(),
                vec![TextEdit {
                    range: Range {
                        start: Position {
                            line: match_end_range.start.line,
                            character: 0,
                        },
                        end: Position {
                            line: match_end_range.start.line,
                            character: 0,
                        },
                    },
                    new_text: "        _ => todo!(),\n".to_string(),
                }],
            );
            changes
        }),
        document_changes: None,
        change_annotations: None,
    };

    CodeAction {
        title: "Add wildcard pattern `_`".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

/// Create warning for redundant pattern
pub fn create_redundant_pattern_diagnostic(
    range: Range,
    pattern_index: usize,
    uri: &Url,
) -> LspDiagnostic {
    LspDiagnostic {
        range,
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String("W0602".to_string())),
        source: Some("verum".to_string()),
        message: format!(
            "unreachable pattern (pattern #{} is covered by earlier patterns)",
            pattern_index + 1
        ),
        related_information: None,
        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
        code_description: Some(CodeDescription {
            href: Url::parse("https://verum-lang.org/errors/W0602")
                .unwrap_or_else(|_| uri.clone()),
        }),
        data: None,
    }
}

/// Create quick fix to remove redundant pattern
pub fn create_remove_redundant_fix(range: Range, uri: &Url) -> CodeAction {
    let edit = WorkspaceEdit {
        changes: Some({
            let mut changes = HashMap::new();
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
        title: "Remove unreachable pattern".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(edit),
        command: None,
        is_preferred: Some(false),
        disabled: None,
        data: None,
    }
}

/// Hover information for match coverage
#[derive(Debug, Clone)]
pub struct MatchCoverageInfo {
    /// Whether the match is exhaustive
    pub is_exhaustive: bool,
    /// Number of patterns
    pub pattern_count: usize,
    /// Number of uncovered cases (if not exhaustive)
    pub uncovered_count: usize,
    /// Number of redundant patterns
    pub redundant_count: usize,
    /// Type being matched
    pub scrutinee_type: Text,
}

impl MatchCoverageInfo {
    /// Generate hover markdown
    pub fn to_hover_markdown(&self) -> String {
        let mut lines = Vec::new();

        lines.push("**Match Expression**".to_string());
        lines.push(format!("Scrutinee type: `{}`", self.scrutinee_type));
        lines.push(format!("Patterns: {}", self.pattern_count));

        if self.is_exhaustive {
            lines.push("✅ **Exhaustive**".to_string());
        } else {
            lines.push(format!(
                "❌ **Non-exhaustive**: {} uncovered case(s)",
                self.uncovered_count
            ));
        }

        if self.redundant_count > 0 {
            lines.push(format!(
                "⚠️ {} redundant pattern(s)",
                self.redundant_count
            ));
        }

        lines.join("\n\n")
    }
}

/// Configuration for exhaustiveness checking in LSP
#[derive(Debug, Clone)]
pub struct ExhaustivenessLspConfig {
    /// Whether to report exhaustiveness errors
    pub report_errors: bool,
    /// Whether to report redundant pattern warnings
    pub report_redundant: bool,
    /// Whether to report all-guarded warnings
    pub report_all_guarded: bool,
    /// Maximum witnesses to show in diagnostics
    pub max_witnesses: usize,
    /// Whether to provide quick fixes
    pub provide_quick_fixes: bool,
    /// Whether to provide hover info
    pub provide_hover_info: bool,
}

impl Default for ExhaustivenessLspConfig {
    fn default() -> Self {
        Self {
            report_errors: true,
            report_redundant: true,
            report_all_guarded: true,
            max_witnesses: 3,
            provide_quick_fixes: true,
            provide_hover_info: true,
        }
    }
}

/// Provider for exhaustiveness-related LSP features
pub struct ExhaustivenessProvider {
    config: ExhaustivenessLspConfig,
}

impl ExhaustivenessProvider {
    /// Create a new provider with default configuration
    pub fn new() -> Self {
        Self {
            config: ExhaustivenessLspConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: ExhaustivenessLspConfig) -> Self {
        Self { config }
    }

    /// Get quick fixes for an exhaustiveness diagnostic
    pub fn get_quick_fixes(
        &self,
        diagnostic: &ExhaustivenessDiagnostic,
        match_end_range: Range,
    ) -> Vec<CodeAction> {
        if !self.config.provide_quick_fixes {
            return Vec::new();
        }

        let mut fixes = Vec::new();

        if !diagnostic.is_exhaustive {
            fixes.push(create_add_patterns_fix(diagnostic, match_end_range));
            fixes.push(create_add_wildcard_fix(diagnostic, match_end_range));
        }

        fixes
    }

    /// Get hover information for a match expression
    pub fn get_hover_info(&self, info: &MatchCoverageInfo) -> Option<Hover> {
        if !self.config.provide_hover_info {
            return None;
        }

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info.to_hover_markdown(),
            }),
            range: None,
        })
    }
}

impl Default for ExhaustivenessProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// REAL-TIME INCREMENTAL EXHAUSTIVENESS CHECKING
// ============================================================
//
// This section provides APIs for incremental exhaustiveness checking
// that integrates with the type checker's incremental infrastructure.

use std::sync::Arc;
use parking_lot::RwLock;
use std::time::{Duration, Instant};

/// Represents the exhaustiveness state for a single match expression
#[derive(Debug, Clone)]
pub struct MatchExhaustivenessState {
    /// Unique ID for this match (e.g., hash of location)
    pub match_id: u64,
    /// Location in source
    pub range: Range,
    /// Current exhaustiveness result
    pub result: MatchExhaustivenessResult,
    /// Last check timestamp
    pub last_checked: Instant,
    /// Whether re-check is needed
    pub dirty: bool,
}

/// Result of exhaustiveness check for a match
#[derive(Debug, Clone)]
pub struct MatchExhaustivenessResult {
    /// Is the match exhaustive?
    pub is_exhaustive: bool,
    /// Uncovered witnesses
    pub witnesses: List<Text>,
    /// Redundant pattern indices
    pub redundant: List<usize>,
    /// Range overlap warnings
    pub range_overlaps: List<RangeOverlapWarning>,
    /// Active pattern optimization hints
    pub optimization_hints: Option<ActivePatternOptHint>,
    /// All patterns are guarded
    pub all_guarded: bool,
}

/// Warning for range pattern overlap
#[derive(Debug, Clone)]
pub struct RangeOverlapWarning {
    /// First pattern index
    pub first_pattern: usize,
    /// Second pattern index (overlapping)
    pub second_pattern: usize,
    /// Start of overlap range
    pub overlap_start: i128,
    /// End of overlap range
    pub overlap_end: i128,
    /// Is the second pattern completely redundant?
    pub is_redundant: bool,
}

impl RangeOverlapWarning {
    /// Convert to LSP diagnostic
    pub fn to_lsp_diagnostic(&self, pattern_range: Range, uri: &Url) -> LspDiagnostic {
        let severity = if self.is_redundant {
            DiagnosticSeverity::WARNING
        } else {
            DiagnosticSeverity::HINT
        };

        let code = if self.is_redundant { "W0607" } else { "W0606" };

        let message = if self.is_redundant {
            format!(
                "range pattern is completely covered by pattern {} (range {}..={})",
                self.first_pattern + 1,
                self.overlap_start,
                self.overlap_end
            )
        } else if self.overlap_start == self.overlap_end {
            format!(
                "patterns {} and {} both match value {}",
                self.first_pattern + 1,
                self.second_pattern + 1,
                self.overlap_start
            )
        } else {
            format!(
                "patterns {} and {} overlap on range {}..={}",
                self.first_pattern + 1,
                self.second_pattern + 1,
                self.overlap_start,
                self.overlap_end
            )
        };

        LspDiagnostic {
            range: pattern_range,
            severity: Some(severity),
            code: Some(NumberOrString::String(code.to_string())),
            source: Some("verum".to_string()),
            message,
            related_information: None,
            tags: if self.is_redundant {
                Some(vec![DiagnosticTag::UNNECESSARY])
            } else {
                None
            },
            code_description: Some(CodeDescription {
                href: Url::parse(&format!("https://verum-lang.org/errors/{}", code))
                    .unwrap_or_else(|_| uri.clone()),
            }),
            data: None,
        }
    }
}

/// Hint for active pattern optimization
#[derive(Debug, Clone)]
pub struct ActivePatternOptHint {
    /// Patterns that are called multiple times
    pub cacheable: List<Text>,
    /// Estimated evaluations saved
    pub savings: usize,
}

/// Incremental exhaustiveness tracker for an entire document
pub struct DocumentExhaustivenessTracker {
    /// Map from match ID to state
    matches: Arc<RwLock<HashMap<u64, MatchExhaustivenessState>>>,
    /// Document URI
    uri: Url,
    /// Configuration
    config: ExhaustivenessLspConfig,
    /// Debounce duration for re-checks
    debounce: Duration,
}

impl DocumentExhaustivenessTracker {
    /// Create a new tracker for a document
    pub fn new(uri: Url) -> Self {
        Self {
            matches: Arc::new(RwLock::new(HashMap::new())),
            uri,
            config: ExhaustivenessLspConfig::default(),
            debounce: Duration::from_millis(100),
        }
    }

    /// Create with custom configuration
    pub fn with_config(uri: Url, config: ExhaustivenessLspConfig, debounce_ms: u64) -> Self {
        Self {
            matches: Arc::new(RwLock::new(HashMap::new())),
            uri,
            config,
            debounce: Duration::from_millis(debounce_ms),
        }
    }

    /// Register a match expression for tracking
    pub fn register_match(&self, match_id: u64, range: Range) {
        let mut matches = self.matches.write();
        matches.insert(match_id, MatchExhaustivenessState {
            match_id,
            range,
            result: MatchExhaustivenessResult {
                is_exhaustive: true, // Assume exhaustive until checked
                witnesses: List::new(),
                redundant: List::new(),
                range_overlaps: List::new(),
                optimization_hints: None,
                all_guarded: false,
            },
            last_checked: Instant::now(),
            dirty: true,
        });
    }

    /// Mark a match as dirty (needs re-check)
    pub fn mark_dirty(&self, match_id: u64) {
        let mut matches = self.matches.write();
        if let Some(state) = matches.get_mut(&match_id) {
            state.dirty = true;
        }
    }

    /// Mark all matches in a range as dirty
    pub fn mark_range_dirty(&self, changed_range: Range) {
        let mut matches = self.matches.write();
        for state in matches.values_mut() {
            // Check if the match overlaps with the changed range
            if ranges_overlap(&state.range, &changed_range) {
                state.dirty = true;
            }
        }
    }

    /// Update the result for a match
    pub fn update_result(&self, match_id: u64, result: MatchExhaustivenessResult) {
        let mut matches = self.matches.write();
        if let Some(state) = matches.get_mut(&match_id) {
            state.result = result;
            state.last_checked = Instant::now();
            state.dirty = false;
        }
    }

    /// Get all matches that need re-checking
    pub fn get_dirty_matches(&self) -> Vec<u64> {
        let matches = self.matches.read();
        let now = Instant::now();
        matches
            .values()
            .filter(|s| s.dirty && now.duration_since(s.last_checked) > self.debounce)
            .map(|s| s.match_id)
            .collect()
    }

    /// Get all diagnostics for the document
    pub fn get_all_diagnostics(&self) -> Vec<LspDiagnostic> {
        let matches = self.matches.read();
        let mut diagnostics = Vec::new();

        for state in matches.values() {
            let diag = ExhaustivenessDiagnostic {
                range: state.range,
                is_exhaustive: state.result.is_exhaustive,
                uncovered_cases: state.result.witnesses.clone(),
                redundant_patterns: state.result.redundant.clone(),
                all_guarded: state.result.all_guarded,
                uri: self.uri.clone(),
            };

            diagnostics.extend(diag.to_lsp_diagnostics());

            // Add range overlap warnings
            for overlap in state.result.range_overlaps.iter() {
                diagnostics.push(overlap.to_lsp_diagnostic(state.range, &self.uri));
            }
        }

        diagnostics
    }

    /// Clear all tracked matches
    pub fn clear(&self) {
        let mut matches = self.matches.write();
        matches.clear();
    }

    /// Remove matches that are no longer in the document
    pub fn gc_stale_matches(&self, valid_match_ids: &[u64]) {
        let mut matches = self.matches.write();
        matches.retain(|id, _| valid_match_ids.contains(id));
    }
}

/// Check if two ranges overlap
fn ranges_overlap(a: &Range, b: &Range) -> bool {
    a.start.line <= b.end.line && b.start.line <= a.end.line
}

/// Compute a hash for a match expression location
pub fn compute_match_id(uri: &Url, start_line: u32, start_col: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    uri.as_str().hash(&mut hasher);
    start_line.hash(&mut hasher);
    start_col.hash(&mut hasher);
    hasher.finish()
}

/// Bridge function to convert verum_types::ExhaustivenessResult to LSP format
pub fn convert_exhaustiveness_result(
    result: &verum_types::exhaustiveness::ExhaustivenessResult,
) -> MatchExhaustivenessResult {
    let witnesses: List<Text> = result.uncovered_witnesses
        .iter()
        .map(|w| Text::from(format!("{}", w)))
        .collect();

    let range_overlaps = if let Some(ref analysis) = result.range_overlaps {
        analysis.overlaps.iter().map(|o| RangeOverlapWarning {
            first_pattern: o.first_pattern_index,
            second_pattern: o.second_pattern_index,
            overlap_start: o.overlap.start,
            overlap_end: o.overlap.end,
            is_redundant: o.is_redundant,
        }).collect()
    } else {
        List::new()
    };

    MatchExhaustivenessResult {
        is_exhaustive: result.is_exhaustive,
        witnesses,
        redundant: result.redundant_patterns.clone(),
        range_overlaps,
        optimization_hints: None, // Set by caller if optimization analysis done
        all_guarded: result.all_guarded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_uri() -> Url {
        Url::parse("file:///test.vr").unwrap()
    }

    #[test]
    fn test_exhaustive_diagnostic() {
        let diag = ExhaustivenessDiagnostic {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 10 },
            },
            is_exhaustive: true,
            uncovered_cases: List::new(),
            redundant_patterns: List::new(),
            all_guarded: false,
            uri: test_uri(),
        };

        let lsp_diags = diag.to_lsp_diagnostics();
        assert!(lsp_diags.is_empty());
    }

    #[test]
    fn test_non_exhaustive_diagnostic() {
        let diag = ExhaustivenessDiagnostic {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 10 },
            },
            is_exhaustive: false,
            uncovered_cases: List::from_iter([Text::from("None")]),
            redundant_patterns: List::new(),
            all_guarded: false,
            uri: test_uri(),
        };

        let lsp_diags = diag.to_lsp_diagnostics();
        assert_eq!(lsp_diags.len(), 1);
        assert!(lsp_diags[0].message.contains("None"));
        assert_eq!(
            lsp_diags[0].code,
            Some(NumberOrString::String("E0601".to_string()))
        );
    }

    #[test]
    fn test_all_guarded_warning() {
        let diag = ExhaustivenessDiagnostic {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 0, character: 10 },
            },
            is_exhaustive: true,
            uncovered_cases: List::new(),
            redundant_patterns: List::new(),
            all_guarded: true,
            uri: test_uri(),
        };

        let lsp_diags = diag.to_lsp_diagnostics();
        assert_eq!(lsp_diags.len(), 1);
        assert_eq!(
            lsp_diags[0].code,
            Some(NumberOrString::String("W0603".to_string()))
        );
    }

    #[test]
    fn test_hover_markdown() {
        let info = MatchCoverageInfo {
            is_exhaustive: true,
            pattern_count: 3,
            uncovered_count: 0,
            redundant_count: 0,
            scrutinee_type: Text::from("Maybe<Int>"),
        };

        let markdown = info.to_hover_markdown();
        assert!(markdown.contains("Exhaustive"));
        assert!(markdown.contains("Maybe<Int>"));
    }

    #[test]
    fn test_redundant_diagnostic() {
        let diag = create_redundant_pattern_diagnostic(
            Range {
                start: Position { line: 5, character: 0 },
                end: Position { line: 5, character: 20 },
            },
            2,
            &test_uri(),
        );

        assert!(diag.message.contains("unreachable"));
        assert!(diag.tags.unwrap().contains(&DiagnosticTag::UNNECESSARY));
    }
}
