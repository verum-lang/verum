//! Core diagnostic types for the Verum compiler.
//!

//! This module provides the fundamental building blocks for compiler diagnostics:
//! spans, labels, severity levels, and the main diagnostic structure.

use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::{List, Text};

// Use LineColSpan from verum_common for diagnostics
use verum_common::span::LineColSpan;

/// Severity level of a diagnostic.
///
/// Variant order (Help < Note < Warning < Error) is the ascending
/// severity ladder consumed by `PartialOrd`/`Ord` derives — used
/// by aggregators that surface "the highest severity in this
/// session".  The numeric `priority` field on [`SeverityMeta`]
/// runs in the *opposite* direction (1 = Error, 4 = Help) to
/// match the historical emitter sort key (errors first); the two
/// orderings are inverse and the relationship is pinned in
/// `meta_pin_severity_priority_inverse_of_ord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Helpful suggestion for fixing an issue
    Help,
    /// Informational note providing context
    Note,
    /// Warning about potential issues
    Warning,
    /// Critical error that prevents compilation
    Error,
}

/// Per-variant projection for [`Severity`].  Single source of
/// truth replacing the five hardcoded match arms that previously
/// scattered the per-variant data across `diagnostic.rs` (Display
/// impl, is_* predicates, accumulator-bucket dispatch) and
/// `emitter.rs` (name strings, priority u8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeverityMeta {
    /// Canonical lower-case wire form — matches the `Display`
    /// impl output and the JSON emitter's `severity` field.
    pub name: &'static str,
    /// Sort key for emitters: 1 (Error) → 4 (Help).  Lower
    /// values surface earlier; inverse polarity to `Ord` on the
    /// enum itself, pinned via the drift test.
    pub priority: u8,
    /// Whether this severity *blocks* compilation — Error
    /// singleton.  Distinguishes hard fail from advisory bands.
    pub is_blocking: bool,
    /// Whether this severity reports a *problem* in user code
    /// (Error + Warning).  False for the *informational* band
    /// (Note + Help) which exists to add context rather than to
    /// flag an issue.  Pinned partition.
    pub is_problem: bool,
    /// Whether this severity is *advisory* (Note + Help) — the
    /// negation of `is_problem`.  Carried for explicit
    /// classifier-side dispatch.
    pub is_advisory: bool,
}

impl Severity {
    /// All variants in `Display`-name-priority order — Error
    /// first so emitters that sort by ALL[i] ordering surface
    /// errors before lower-priority noise.
    pub const ALL: &'static [Self] = &[
        Self::Error,
        Self::Warning,
        Self::Note,
        Self::Help,
    ];

    /// Static fact-pack — single source of truth for the
    /// per-variant dispatch surface.
    pub const fn meta(self) -> SeverityMeta {
        match self {
            Severity::Error => SeverityMeta {
                name: "error",
                priority: 1,
                is_blocking: true,
                is_problem: true,
                is_advisory: false,
            },
            Severity::Warning => SeverityMeta {
                name: "warning",
                priority: 2,
                is_blocking: false,
                is_problem: true,
                is_advisory: false,
            },
            Severity::Note => SeverityMeta {
                name: "note",
                priority: 3,
                is_blocking: false,
                is_problem: false,
                is_advisory: true,
            },
            Severity::Help => SeverityMeta {
                name: "help",
                priority: 4,
                is_blocking: false,
                is_problem: false,
                is_advisory: true,
            },
        }
    }

    /// Canonical lower-case wire form via `meta()`.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Inverse of `as_str` — recover the severity from the wire
    /// form.  Returns `None` for unknown strings.
    pub fn from_str(s: &str) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            let v = Self::ALL[i];
            if v.meta().name.as_bytes() == s.as_bytes() {
                return Some(v);
            }
            i += 1;
        }
        None
    }

    /// Emitter sort key (1=Error, 2=Warning, 3=Note, 4=Help).
    /// Inverse polarity to `Ord` on the enum itself.
    #[inline]
    pub const fn priority(self) -> u8 {
        self.meta().priority
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Re-export LineColSpan as Span for backward compatibility
pub type Span = LineColSpan;

/// A label attached to a span with a message
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpanLabel {
    /// The span this label points to
    pub span: Span,
    /// The message for this label
    pub message: Text,
    /// Is this the primary label?
    pub is_primary: bool,
}

impl SpanLabel {
    /// Create a new primary label
    pub fn primary(span: Span, message: impl Into<Text>) -> Self {
        Self {
            span,
            message: message.into(),
            is_primary: true,
        }
    }

    /// Create a new secondary label
    pub fn secondary(span: Span, message: impl Into<Text>) -> Self {
        Self {
            span,
            message: message.into(),
            is_primary: false,
        }
    }
}

/// A simple label without span information (for notes and help messages)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    /// The message
    pub message: Text,
}

impl Label {
    pub fn new(message: impl Into<Text>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Source location for machine-readable output
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: Text,
    pub line: usize,
    pub column: usize,
    pub length: usize,
}

impl From<&Span> for SourceLocation {
    fn from(span: &Span) -> Self {
        Self {
            file: span.file.clone(),
            line: span.line,
            column: span.column,
            length: span.length(),
        }
    }
}

/// A complete diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity level
    severity: Severity,
    /// Error/warning code (e.g., "E0308")
    code: Option<Text>,
    /// Main diagnostic message
    message: Text,
    /// Primary span labels (usually one)
    primary_labels: List<SpanLabel>,
    /// Secondary span labels (additional context)
    secondary_labels: List<SpanLabel>,
    /// Note messages (context and explanation)
    notes: List<Label>,
    /// Help messages (actionable suggestions)
    helps: List<Label>,
    /// Child diagnostics (nested errors/warnings)
    children: List<Diagnostic>,
    /// Unique identifier for deduplication
    #[serde(skip)]
    dedup_key: Option<Text>,
    /// Fixable flag indicating if this diagnostic has applicable suggestions
    is_fixable: bool,
    /// Related file paths for multi-file diagnostics
    related_files: List<Text>,
    /// Documentation URL for extended explanation
    doc_url: Option<Text>,
    /// Suggested actions that can be auto-applied
    suggested_fixes: List<SuggestedFix>,
    /// Synthetic-expansion provenance chain (#284-Renderer, task #287).
    ///

    /// When the diagnostic's primary span lands in synthetic source
    /// (macro / @derive / monomorphization / @delegate output), the
    /// builder calls `with_expansion_chain` to attach the human-
    /// readable labels of every expansion layer traversed by
    /// `Span::resolve_to_user_source`. The renderer surfaces this
    /// chain after the location header so users see "in @derive
    /// expansion → in macro expansion" instead of an opaque
    /// `<synthetic>:1:1` location.
    ///

    /// Empty for diagnostics whose span is already user-source —
    /// the most common case at the steady state.
    #[serde(default, skip_serializing_if = "List::is_empty")]
    expansion_chain: List<Text>,
}

/// A fix that can be automatically applied
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedFix {
    /// Human-readable description of the fix
    pub message: Text,
    /// The span to replace
    pub span: Span,
    /// The replacement text
    pub replacement: Text,
    /// Whether this fix is safe to auto-apply
    pub is_machine_applicable: bool,
}

impl SuggestedFix {
    /// Create a new suggested fix
    pub fn new(
        message: impl Into<Text>,
        span: Span,
        replacement: impl Into<Text>,
        is_machine_applicable: bool,
    ) -> Self {
        Self {
            message: message.into(),
            span,
            replacement: replacement.into(),
            is_machine_applicable,
        }
    }

    /// Create a machine-applicable fix
    pub fn machine_applicable(
        message: impl Into<Text>,
        span: Span,
        replacement: impl Into<Text>,
    ) -> Self {
        Self::new(message, span, replacement, true)
    }

    /// Create a fix that requires human verification
    pub fn maybe_applicable(
        message: impl Into<Text>,
        span: Span,
        replacement: impl Into<Text>,
    ) -> Self {
        Self::new(message, span, replacement, false)
    }
}

impl Diagnostic {
    /// Get the severity level
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// Get the error code
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Get the main message
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    /// Get all primary labels
    pub fn primary_labels(&self) -> &[SpanLabel] {
        &self.primary_labels
    }

    /// Get all secondary labels
    pub fn secondary_labels(&self) -> &[SpanLabel] {
        &self.secondary_labels
    }

    /// Get all notes
    pub fn notes(&self) -> &[Label] {
        &self.notes
    }

    /// Get all help messages
    pub fn helps(&self) -> &[Label] {
        &self.helps
    }

    /// Get all child diagnostics
    pub fn children(&self) -> &[Diagnostic] {
        &self.children
    }

    /// Get the synthetic-expansion provenance chain (#287). Each
    /// entry is a human-readable label like `"@derive expansion"`
    /// or `"macro expansion"`, ordered leaf-to-root (deepest
    /// synthetic first, ending at the user-visible call site).
    /// Empty when the primary span is already user-source.
    pub fn expansion_chain(&self) -> &[Text] {
        &self.expansion_chain
    }

    /// Check if this is an error
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Check if this is a warning
    pub fn is_warning(&self) -> bool {
        self.severity == Severity::Warning
    }

    /// Check if this is a note
    pub fn is_note(&self) -> bool {
        self.severity == Severity::Note
    }

    /// Check if this is a help
    pub fn is_help(&self) -> bool {
        self.severity == Severity::Help
    }

    /// Get the primary span (first primary label's span)
    pub fn primary_span(&self) -> Option<&Span> {
        self.primary_labels.first().map(|l| &l.span)
    }

    /// Get all spans (primary and secondary)
    pub fn all_spans(&self) -> impl Iterator<Item = &Span> {
        self.primary_labels
            .iter()
            .chain(self.secondary_labels.iter())
            .map(|l| &l.span)
    }

    /// Check if this diagnostic has any suggested fixes
    pub fn is_fixable(&self) -> bool {
        self.is_fixable || !self.suggested_fixes.is_empty()
    }

    /// Get all suggested fixes
    pub fn suggested_fixes(&self) -> &[SuggestedFix] {
        &self.suggested_fixes
    }

    /// Get the documentation URL if available
    pub fn doc_url(&self) -> Option<&str> {
        self.doc_url.as_deref()
    }

    /// Get related file paths
    pub fn related_files(&self) -> &[Text] {
        &self.related_files
    }

    /// Get the deduplication key
    pub fn dedup_key(&self) -> Option<&str> {
        self.dedup_key.as_deref()
    }

    /// Compute a deduplication key from the diagnostic content
    pub fn compute_dedup_key(&self) -> Text {
        format!(
            "{}:{}:{}:{}",
            self.severity,
            self.code.as_deref().unwrap_or(""),
            self.primary_span()
                .map(|s| format!("{}:{}:{}", s.file, s.line, s.column))
                .unwrap_or_default(),
            self.message
        )
        .into()
    }

    /// Get machine-applicable fixes that can be auto-applied
    pub fn machine_applicable_fixes(&self) -> impl Iterator<Item = &SuggestedFix> {
        self.suggested_fixes
            .iter()
            .filter(|f| f.is_machine_applicable)
    }

    /// Format a short summary for terminal display
    pub fn short_summary(&self) -> Text {
        let location = self
            .primary_span()
            .map(|s| format!("{}:{}:{}", s.file, s.line, s.column))
            .unwrap_or_else(|| "<no location>".to_string());
        format!(
            "{}{}: {} at {}",
            self.severity,
            self.code
                .as_ref()
                .map(|c| format!("[{}]", c))
                .unwrap_or_default(),
            self.message,
            location
        )
        .into()
    }

    /// Get the total count of all diagnostics including children
    pub fn total_count(&self) -> usize {
        1 + self.children.iter().map(|c| c.total_count()).sum::<usize>()
    }

    /// Get the count of errors (including children)
    pub fn error_count(&self) -> usize {
        let self_count = if self.is_error() { 1 } else { 0 };
        self_count + self.children.iter().map(|c| c.error_count()).sum::<usize>()
    }

    /// Get the count of warnings (including children)
    pub fn warning_count(&self) -> usize {
        let self_count = if self.is_warning() { 1 } else { 0 };
        self_count
            + self
                .children
                .iter()
                .map(|c| c.warning_count())
                .sum::<usize>()
    }

    // ==================== Convenience Constructors ====================

    /// Create a new error diagnostic with message, span, and code
    pub fn new_error(message: impl Into<Text>, span: Span, code: impl Into<Text>) -> Self {
        DiagnosticBuilder::error()
            .message(message)
            .span(span)
            .code(code)
            .build()
    }

    /// Create a new warning diagnostic with message, span, and code
    pub fn new_warning(message: impl Into<Text>, span: Span, code: impl Into<Text>) -> Self {
        DiagnosticBuilder::warning()
            .message(message)
            .span(span)
            .code(code)
            .build()
    }

    /// Create a new note diagnostic with message and span
    pub fn new_note(message: impl Into<Text>, span: Span) -> Self {
        DiagnosticBuilder::note_diag()
            .message(message)
            .span(span)
            .build()
    }

    /// Create a new help diagnostic with message and span
    pub fn new_help(message: impl Into<Text>, span: Span) -> Self {
        DiagnosticBuilder::help_diag()
            .message(message)
            .span(span)
            .build()
    }

    /// Create a simple error without span
    pub fn simple_error(message: impl Into<Text>) -> Self {
        DiagnosticBuilder::error().message(message).build()
    }

    /// Create a simple warning without span
    pub fn simple_warning(message: impl Into<Text>) -> Self {
        DiagnosticBuilder::warning().message(message).build()
    }
}

/// Builder for constructing diagnostics
pub struct DiagnosticBuilder {
    severity: Severity,
    code: Option<Text>,
    message: Text,
    primary_labels: List<SpanLabel>,
    secondary_labels: List<SpanLabel>,
    notes: List<Label>,
    helps: List<Label>,
    children: List<Diagnostic>,
    is_fixable: bool,
    related_files: List<Text>,
    doc_url: Option<Text>,
    suggested_fixes: List<SuggestedFix>,
    expansion_chain: List<Text>,
}

impl DiagnosticBuilder {
    /// Create a new error diagnostic
    pub fn error() -> Self {
        Self::new(Severity::Error)
    }

    /// Create a new warning diagnostic
    pub fn warning() -> Self {
        Self::new(Severity::Warning)
    }

    /// Create a new note diagnostic
    pub fn note_diag() -> Self {
        Self::new(Severity::Note)
    }

    /// Create a new help diagnostic
    pub fn help_diag() -> Self {
        Self::new(Severity::Help)
    }

    /// Create a new diagnostic with the given severity
    pub fn new(severity: Severity) -> Self {
        Self {
            severity,
            code: None,
            message: Text::new(),
            primary_labels: List::new(),
            secondary_labels: List::new(),
            notes: List::new(),
            helps: List::new(),
            children: List::new(),
            is_fixable: false,
            related_files: List::new(),
            doc_url: None,
            suggested_fixes: List::new(),
            expansion_chain: List::new(),
        }
    }

    /// Set the error code
    pub fn code(mut self, code: impl Into<Text>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Set the main message
    pub fn message(mut self, message: impl Into<Text>) -> Self {
        self.message = message.into();
        self
    }

    /// Add a primary span label
    pub fn span(mut self, span: Span) -> Self {
        self.primary_labels
            .push(SpanLabel::primary(span, Text::new()));
        self
    }

    /// Add a primary span label with message
    pub fn span_label(mut self, span: Span, message: impl Into<Text>) -> Self {
        self.primary_labels.push(SpanLabel::primary(span, message));
        self
    }

    /// Add a label to the most recent span
    pub fn label(mut self, message: impl Into<Text>) -> Self {
        if let Some(label) = self.primary_labels.last_mut() {
            label.message = message.into();
        }
        self
    }

    /// Add a secondary span label
    pub fn secondary_span(mut self, span: Span, message: impl Into<Text>) -> Self {
        self.secondary_labels
            .push(SpanLabel::secondary(span, message));
        self
    }

    /// Add a note
    pub fn add_note(mut self, message: impl Into<Text>) -> Self {
        self.notes.push(Label::new(message));
        self
    }

    /// Add a help message
    pub fn help(mut self, message: impl Into<Text>) -> Self {
        self.helps.push(Label::new(message));
        self
    }

    /// Add a child diagnostic
    pub fn child(mut self, child: Diagnostic) -> Self {
        self.children.push(child);
        self
    }

    /// Mark this diagnostic as fixable
    pub fn fixable(mut self) -> Self {
        self.is_fixable = true;
        self
    }

    /// Add a related file
    pub fn related_file(mut self, file: impl Into<Text>) -> Self {
        self.related_files.push(file.into());
        self
    }

    /// Set the documentation URL
    pub fn doc_url(mut self, url: impl Into<Text>) -> Self {
        self.doc_url = Some(url.into());
        self
    }

    /// Add a suggested fix
    pub fn suggested_fix(mut self, fix: SuggestedFix) -> Self {
        self.suggested_fixes.push(fix);
        self.is_fixable = true;
        self
    }

    /// Add a machine-applicable fix
    pub fn fix_at(
        mut self,
        message: impl Into<Text>,
        span: Span,
        replacement: impl Into<Text>,
    ) -> Self {
        self.suggested_fixes
            .push(SuggestedFix::machine_applicable(message, span, replacement));
        self.is_fixable = true;
        self
    }

    /// Attach a synthetic-expansion provenance chain (#287).
    /// Callers walk `Span::resolve_to_user_source(|fid|
    /// session.synthetic_origin(fid))` and pass the chain's
    /// `SyntheticKind::label()` outputs here so the renderer can
    /// surface "in @derive expansion → in macro expansion" labels
    /// after the location header.
    ///

    /// Empty chain (the default) is the user-source case — no
    /// rendering overhead, behaviour bit-identical to pre-#287.
    pub fn expansion_chain<I, S>(mut self, chain: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<Text>,
    {
        for label in chain {
            self.expansion_chain.push(label.into());
        }
        self
    }

    /// Build the diagnostic
    pub fn build(self) -> Diagnostic {
        Diagnostic {
            severity: self.severity,
            code: self.code,
            message: self.message,
            primary_labels: self.primary_labels,
            secondary_labels: self.secondary_labels,
            notes: self.notes,
            helps: self.helps,
            children: self.children,
            dedup_key: None,
            is_fixable: self.is_fixable,
            related_files: self.related_files,
            doc_url: self.doc_url,
            suggested_fixes: self.suggested_fixes,
            expansion_chain: self.expansion_chain,
        }
    }
}

/// Utility for word-wrapping messages
pub struct MessageFormatter {
    max_width: usize,
}

impl MessageFormatter {
    /// Create a new formatter with the given max width
    pub fn new(max_width: usize) -> Self {
        Self { max_width }
    }

    /// Default formatter with 80-column width
    pub fn default_width() -> Self {
        Self::new(80)
    }

    /// Wrap text to the configured width
    pub fn wrap(&self, text: &str) -> Text {
        let mut result = Text::new();
        let mut current_line_len = 0;

        for word in text.split_whitespace() {
            let word_len = word.len();

            if current_line_len + word_len + 1 > self.max_width && current_line_len > 0 {
                result.push('\n');
                current_line_len = 0;
            }

            if current_line_len > 0 {
                result.push(' ');
                current_line_len += 1;
            }

            result.push_str(word);
            current_line_len += word_len;
        }

        result
    }

    /// Wrap text with an indentation prefix on continuation lines
    pub fn wrap_with_indent(&self, text: &str, indent: &str) -> Text {
        let mut result = Text::new();
        let mut current_line_len = 0;
        let indent_len = indent.len();
        let effective_width = self.max_width.saturating_sub(indent_len);

        for word in text.split_whitespace() {
            let word_len = word.len();

            if current_line_len + word_len + 1 > effective_width && current_line_len > 0 {
                result.push('\n');
                result.push_str(indent);
                current_line_len = 0;
            }

            if current_line_len > 0 {
                result.push(' ');
                current_line_len += 1;
            }

            result.push_str(word);
            current_line_len += word_len;
        }

        result
    }

    /// Format a message with proper prefix and continuation indentation
    pub fn format_message(&self, prefix: &str, message: &str) -> Text {
        let prefix_len = prefix.len();
        let indent = " ".repeat(prefix_len);
        let first_line_width = self.max_width.saturating_sub(prefix_len);

        let mut result = Text::new();
        result.push_str(prefix);

        let mut current_line_len = 0;
        let mut first_word = true;

        for word in message.split_whitespace() {
            let word_len = word.len();
            let width = if result.lines().len() == 1 {
                first_line_width
            } else {
                self.max_width.saturating_sub(indent.len())
            };

            if current_line_len + word_len + 1 > width && current_line_len > 0 {
                result.push('\n');
                result.push_str(&indent);
                current_line_len = 0;
                first_word = true;
            }

            if !first_word {
                result.push(' ');
                current_line_len += 1;
            }

            result.push_str(word);
            current_line_len += word_len;
            first_word = false;
        }

        result
    }
}

/// Collect and aggregate diagnostics
#[derive(Debug, Clone, Default)]
pub struct DiagnosticCollector {
    diagnostics: List<Diagnostic>,
    error_count: usize,
    warning_count: usize,
    note_count: usize,
}

impl DiagnosticCollector {
    /// Create a new empty collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a diagnostic to the collector
    pub fn add(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Error => self.error_count += 1,
            Severity::Warning => self.warning_count += 1,
            Severity::Note | Severity::Help => self.note_count += 1,
        }
        self.diagnostics.push(diagnostic);
    }

    /// Add an error diagnostic
    pub fn error(&mut self, message: impl Into<Text>, span: Span, code: impl Into<Text>) {
        self.add(Diagnostic::new_error(message, span, code));
    }

    /// Add a warning diagnostic
    pub fn warning(&mut self, message: impl Into<Text>, span: Span, code: impl Into<Text>) {
        self.add(Diagnostic::new_warning(message, span, code));
    }

    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Check if there are any warnings
    pub fn has_warnings(&self) -> bool {
        self.warning_count > 0
    }

    /// Get the error count
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Get the warning count
    pub fn warning_count(&self) -> usize {
        self.warning_count
    }

    /// Get all diagnostics
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Take all diagnostics, consuming the collector
    pub fn take(self) -> List<Diagnostic> {
        self.diagnostics
    }

    /// Get a summary message
    pub fn summary(&self) -> Text {
        let mut parts = List::new();
        if self.error_count > 0 {
            parts.push(format!(
                "{} error{}",
                self.error_count,
                if self.error_count == 1 { "" } else { "s" }
            ));
        }
        if self.warning_count > 0 {
            parts.push(format!(
                "{} warning{}",
                self.warning_count,
                if self.warning_count == 1 { "" } else { "s" }
            ));
        }
        if parts.is_empty() {
            "no errors".into()
        } else {
            parts.join(", ")
        }
    }

    /// Deduplicate diagnostics based on their dedup key
    pub fn deduplicate(&mut self) {
        let mut seen: std::collections::HashSet<Text> = std::collections::HashSet::new();
        self.diagnostics.retain(|d| {
            let key = d.compute_dedup_key();
            seen.insert(key)
        });

        // Recalculate counts
        self.error_count = self.diagnostics.iter().filter(|d| d.is_error()).count();
        self.warning_count = self.diagnostics.iter().filter(|d| d.is_warning()).count();
        self.note_count = self
            .diagnostics
            .iter()
            .filter(|d| d.is_note() || d.is_help())
            .count();
    }

    /// Sort diagnostics by severity (errors first) then by file/line
    pub fn sort(&mut self) {
        self.diagnostics.sort_by(|a, b| {
            // Errors before warnings before notes
            b.severity().cmp(&a.severity()).then_with(|| {
                // Then by file
                let a_loc = a.primary_span();
                let b_loc = b.primary_span();
                match (a_loc, b_loc) {
                    (Some(a), Some(b)) => a
                        .file
                        .cmp(&b.file)
                        .then(a.line.cmp(&b.line))
                        .then(a.column.cmp(&b.column)),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
        });
    }
}

#[cfg(test)]
mod severity_meta_drift_pins {
    use super::*;

    /// Drift-pin: `Severity::meta()` is the single source of
    /// truth for the per-variant dispatch surface previously
    /// scattered across 5 hardcoded match arms.  Pins variant
    /// count, name partition, priority partition, classifier
    /// flags, and the cross-cutting invariants binding them.
    #[test]
    fn meta_pin_severity_round_trip_and_partitions() {
        // 1. Variant count + name uniqueness + round-trip.
        assert_eq!(Severity::ALL.len(), 4);
        let mut seen = std::collections::HashSet::new();
        for s in Severity::ALL {
            let m = s.meta();
            assert!(
                m.name.chars().all(|c| c.is_ascii_lowercase()),
                "{:?}: name not lowercase: {}",
                s,
                m.name,
            );
            assert!(seen.insert(m.name), "{:?}: duplicate name", s);
            assert_eq!(Severity::from_str(m.name), Some(*s));
            assert_eq!(s.as_str(), m.name);
        }
        assert_eq!(Severity::from_str("nope"), None);

        // 2. Display impl agrees with as_str().
        for s in Severity::ALL {
            assert_eq!(format!("{}", s), s.as_str());
        }

        // 3. Priority is dense 1..=4 + unique.
        let mut priorities: Vec<u8> = Severity::ALL.iter().map(|s| s.priority()).collect();
        priorities.sort();
        assert_eq!(priorities, vec![1, 2, 3, 4]);

        // 4. is_blocking — Error singleton.
        let blocking: Vec<_> = Severity::ALL
            .iter()
            .filter(|s| s.meta().is_blocking)
            .copied()
            .collect();
        assert_eq!(blocking, vec![Severity::Error]);

        // 5. is_problem — Error + Warning.
        let problems: Vec<_> = Severity::ALL
            .iter()
            .filter(|s| s.meta().is_problem)
            .copied()
            .collect();
        assert_eq!(problems, vec![Severity::Error, Severity::Warning]);

        // 6. is_advisory — Note + Help.
        let advisory: Vec<_> = Severity::ALL
            .iter()
            .filter(|s| s.meta().is_advisory)
            .copied()
            .collect();
        assert_eq!(advisory, vec![Severity::Note, Severity::Help]);

        // 7. Cross-cutting: is_problem ⊕ is_advisory (every
        //    severity is exactly one of the two — perfect
        //    partition over the 4 variants).
        for s in Severity::ALL {
            let m = s.meta();
            assert!(
                m.is_problem ^ m.is_advisory,
                "{:?}: must flip exactly one of is_problem / is_advisory",
                s
            );
        }

        // 8. is_blocking ⇒ is_problem (a blocking severity is
        //    necessarily problem-class — a friendly hint can't
        //    block compilation).
        for s in Severity::ALL {
            let m = s.meta();
            assert!(
                !m.is_blocking || m.is_problem,
                "{:?}: blocking ⇒ problem",
                s
            );
        }
    }

    /// Drift-pin: `priority()` runs in *inverse* polarity to the
    /// derived `Ord` on the enum.  Severity::Error is the
    /// highest by `Ord` (rightmost variant) but priority 1 (the
    /// lowest = highest emit precedence).  Pinned so a future
    /// reorder of either ordering surfaces here.
    #[test]
    fn meta_pin_severity_priority_inverse_of_ord() {
        // Variant order ascending: Help < Note < Warning < Error.
        let by_ord = {
            let mut v = Severity::ALL.to_vec();
            v.sort();
            v
        };
        assert_eq!(
            by_ord,
            vec![
                Severity::Help,
                Severity::Note,
                Severity::Warning,
                Severity::Error,
            ],
        );

        // Priority ascending: Error(1) < Warning(2) < Note(3) < Help(4).
        let by_priority = {
            let mut v = Severity::ALL.to_vec();
            v.sort_by_key(|s| s.priority());
            v
        };
        assert_eq!(
            by_priority,
            vec![
                Severity::Error,
                Severity::Warning,
                Severity::Note,
                Severity::Help,
            ],
        );

        // The two orderings are reverse of each other — a
        // sequence sorted ascending by Ord and then reversed
        // matches a sequence sorted ascending by priority.
        let mut reversed_ord = by_ord.clone();
        reversed_ord.reverse();
        assert_eq!(reversed_ord, by_priority);
    }
}

#[cfg(test)]
mod expansion_chain_tests {
    use super::*;

    #[test]
    fn default_diagnostic_has_empty_expansion_chain() {
        // Pin: pre-#287 builders produce diagnostics with empty
        // expansion_chain — bit-identical to legacy behaviour.
        let diag = DiagnosticBuilder::error().message("test error").build();
        assert!(
            diag.expansion_chain().is_empty(),
            "no-chain default must produce empty expansion_chain"
        );
    }

    #[test]
    fn with_expansion_chain_preserves_order() {
        // Pin: chain order is leaf-to-root (deepest synthetic
        // first), preserved verbatim from input. Renderer joins
        // with " → " so order matters.
        let chain = vec!["@derive expansion", "macro expansion"];
        let diag = DiagnosticBuilder::error()
            .message("test error")
            .expansion_chain(chain)
            .build();
        let stored = diag.expansion_chain();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].as_str(), "@derive expansion");
        assert_eq!(stored[1].as_str(), "macro expansion");
    }

    #[test]
    fn empty_chain_is_no_op() {
        // Pin: passing an empty iterator leaves the chain empty —
        // no false-positive expansion-chain lines from accidental
        // empty calls.
        let diag = DiagnosticBuilder::error()
            .message("test")
            .expansion_chain(Vec::<&str>::new())
            .build();
        assert!(diag.expansion_chain().is_empty());
    }

    #[test]
    fn chain_accepts_text_and_static_str() {
        // Pin: the IntoIterator<Item: Into<Text>> bound accepts
        // both &'static str and Text — call sites can use either.
        let static_strs = vec!["macro expansion"];
        let text_strs = vec![Text::from("@derive expansion")];

        let diag1 = DiagnosticBuilder::error()
            .message("a")
            .expansion_chain(static_strs)
            .build();
        let diag2 = DiagnosticBuilder::error()
            .message("b")
            .expansion_chain(text_strs)
            .build();

        assert_eq!(diag1.expansion_chain().len(), 1);
        assert_eq!(diag2.expansion_chain().len(), 1);
    }

    #[test]
    fn chain_round_trips_through_serde() {
        // Pin: serde round-trips preserve the expansion_chain.
        // Skipped at empty (skip_serializing_if) so user-source
        // diagnostics stay bit-identical to pre-#287 in JSON.
        let diag = DiagnosticBuilder::error()
            .message("test")
            .expansion_chain(vec!["macro expansion", "@derive expansion"])
            .build();
        let json = serde_json::to_string(&diag).expect("serialize");
        assert!(
            json.contains("expansion_chain"),
            "non-empty chain must serialize"
        );
        let restored: Diagnostic = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.expansion_chain().len(), 2);
        assert_eq!(restored.expansion_chain()[0].as_str(), "macro expansion");
    }

    #[test]
    fn empty_chain_omitted_from_serde() {
        // Pin: empty chain skipped from JSON output (skip_
        // serializing_if). User-source diagnostics produce
        // identical JSON to pre-#287 builds.
        let diag = DiagnosticBuilder::error().message("test").build();
        let json = serde_json::to_string(&diag).expect("serialize");
        assert!(
            !json.contains("expansion_chain"),
            "empty chain must be skipped from JSON"
        );
    }
}
