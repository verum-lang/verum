//! Diagnostics Sub-Context
//!
//! Manages compilation diagnostics, source mapping, and error/warning collection
//! during meta function execution.
//!
//! ## Responsibility
//!
//! - Error and warning collection
//! - Source code mapping (file_id -> source text)
//! - Span mappings for generated code
//! - Line directives for debugging
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_common::{span::LineColSpan, List, Map, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

/// Diagnostics collector for meta function execution
///
/// Collects errors, warnings, and other diagnostics during compile-time
/// evaluation, along with source mapping information for generated code.
#[derive(Debug, Clone, Default)]
pub struct DiagnosticsCollector {
    /// Accumulated diagnostics
    diagnostics: List<Diagnostic>,

    /// Count of errors emitted
    error_count: usize,

    /// Count of warnings emitted
    warning_count: usize,

    /// Source code map (file_id -> source text)
    source_map: Map<u32, Text>,

    /// Source file path for current parsing context
    source_file: Option<Text>,

    /// Input token stream to the current macro
    macro_input: Option<Text>,

    /// Attribute arguments if invoked as attribute macro
    attr_args: Option<Text>,

    /// Stack of generated code scopes
    source_map_scope_stack: List<Text>,

    /// Span mappings (generated span -> generator function)
    span_mappings: List<(LineColSpan, Text)>,

    /// Generated to source span mapping
    generated_to_source_map: Map<Text, LineColSpan>,

    /// Line directives (file, line)
    line_directives: List<(Text, u32)>,

    /// Counter for synthetic span IDs
    next_synthetic_span_id: u64,
}

impl DiagnosticsCollector {
    /// Create a new empty diagnostics collector
    pub fn new() -> Self {
        Self::default()
    }

    // ======== Diagnostic Operations ========

    /// Add a diagnostic
    pub fn add_diagnostic(&mut self, diagnostic: Diagnostic) {
        match diagnostic.severity() {
            Severity::Error => self.error_count += 1,
            Severity::Warning => self.warning_count += 1,
            _ => {}
        }
        self.diagnostics.push(diagnostic);
    }

    /// Add an error diagnostic
    ///
    /// Note: The span is stored as byte offsets. For proper line/column display,
    /// use the source_map to convert byte offsets before final rendering.
    pub fn add_error(&mut self, message: Text, _span: verum_ast::Span) {
        self.error_count += 1;
        let diag = DiagnosticBuilder::error()
            .message(message.to_string())
            .build();
        self.diagnostics.push(diag);
    }

    /// Add a warning diagnostic
    ///
    /// Note: The span is stored as byte offsets. For proper line/column display,
    /// use the source_map to convert byte offsets before final rendering.
    pub fn add_warning(&mut self, message: Text, _span: verum_ast::Span) {
        self.warning_count += 1;
        let diag = DiagnosticBuilder::warning()
            .message(message.to_string())
            .build();
        self.diagnostics.push(diag);
    }

    /// Get all diagnostics
    pub fn diagnostics(&self) -> &List<Diagnostic> {
        &self.diagnostics
    }

    /// Get mutable access to diagnostics
    pub fn diagnostics_mut(&mut self) -> &mut List<Diagnostic> {
        &mut self.diagnostics
    }

    /// Take all diagnostics, clearing the collector
    pub fn take_diagnostics(&mut self) -> List<Diagnostic> {
        let diagnostics = std::mem::take(&mut self.diagnostics);
        self.error_count = 0;
        self.warning_count = 0;
        diagnostics
    }

    /// Get error count
    #[inline]
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Get warning count
    #[inline]
    pub fn warning_count(&self) -> usize {
        self.warning_count
    }

    /// Check if there are any errors
    #[inline]
    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// Clear all diagnostics
    pub fn clear(&mut self) {
        self.diagnostics.clear();
        self.error_count = 0;
        self.warning_count = 0;
    }

    // ======== Source Map Operations ========

    /// Register source text for a file ID
    pub fn register_source(&mut self, file_id: u32, source: Text) {
        self.source_map.insert(file_id, source);
    }

    /// Get source text for a file ID
    pub fn get_source(&self, file_id: u32) -> Option<&Text> {
        self.source_map.get(&file_id)
    }

    /// Get the source map
    pub fn source_map(&self) -> &Map<u32, Text> {
        &self.source_map
    }

    /// Get mutable access to source map
    pub fn source_map_mut(&mut self) -> &mut Map<u32, Text> {
        &mut self.source_map
    }

    // ======== Source File Context ========

    /// Set current source file
    #[inline]
    pub fn set_source_file(&mut self, path: Option<Text>) {
        self.source_file = path;
    }

    /// Get current source file
    #[inline]
    pub fn source_file(&self) -> Option<&Text> {
        self.source_file.as_ref()
    }

    /// Set macro input
    #[inline]
    pub fn set_macro_input(&mut self, input: Option<Text>) {
        self.macro_input = input;
    }

    /// Get macro input
    #[inline]
    pub fn macro_input(&self) -> Option<&Text> {
        self.macro_input.as_ref()
    }

    /// Set attribute arguments
    #[inline]
    pub fn set_attr_args(&mut self, args: Option<Text>) {
        self.attr_args = args;
    }

    /// Get attribute arguments
    #[inline]
    pub fn attr_args(&self) -> Option<&Text> {
        self.attr_args.as_ref()
    }

    // ======== Generated Code Tracking ========

    /// Push a source map scope
    pub fn push_scope(&mut self, scope: Text) {
        self.source_map_scope_stack.push(scope);
    }

    /// Pop a source map scope
    pub fn pop_scope(&mut self) -> Option<Text> {
        self.source_map_scope_stack.pop()
    }

    /// Get current scope stack
    pub fn scope_stack(&self) -> &List<Text> {
        &self.source_map_scope_stack
    }

    /// Add a span mapping
    pub fn add_span_mapping(&mut self, span: LineColSpan, generator: Text) {
        self.span_mappings.push((span, generator));
    }

    /// Get span mappings
    pub fn span_mappings(&self) -> &List<(LineColSpan, Text)> {
        &self.span_mappings
    }

    /// Register generated to source mapping
    pub fn register_generated_span(&mut self, generated_id: Text, source_span: LineColSpan) {
        self.generated_to_source_map.insert(generated_id, source_span);
    }

    /// Get source span for generated code
    pub fn get_source_for_generated(&self, generated_id: &Text) -> Option<&LineColSpan> {
        self.generated_to_source_map.get(generated_id)
    }

    /// Get generated to source map
    pub fn generated_to_source_map(&self) -> &Map<Text, LineColSpan> {
        &self.generated_to_source_map
    }

    // ======== Line Directives ========

    /// Add a line directive
    pub fn add_line_directive(&mut self, file: Text, line: u32) {
        self.line_directives.push((file, line));
    }

    /// Get line directives
    pub fn line_directives(&self) -> &List<(Text, u32)> {
        &self.line_directives
    }

    // ======== Synthetic Spans ========

    /// Generate a unique synthetic span ID
    pub fn gen_synthetic_span_id(&mut self) -> u64 {
        let id = self.next_synthetic_span_id;
        self.next_synthetic_span_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_counts() {
        let mut collector = DiagnosticsCollector::new();
        assert_eq!(collector.error_count(), 0);
        assert_eq!(collector.warning_count(), 0);
        assert!(!collector.has_errors());

        collector.add_error(Text::from("error 1"), verum_ast::Span::dummy());
        assert_eq!(collector.error_count(), 1);
        assert!(collector.has_errors());

        collector.add_warning(Text::from("warning 1"), verum_ast::Span::dummy());
        assert_eq!(collector.warning_count(), 1);
    }

    #[test]
    fn test_source_map() {
        let mut collector = DiagnosticsCollector::new();
        collector.register_source(1, Text::from("fn main() {}"));
        assert_eq!(
            collector.get_source(1),
            Some(&Text::from("fn main() {}"))
        );
        assert!(collector.get_source(2).is_none());
    }

    #[test]
    fn test_take_diagnostics() {
        let mut collector = DiagnosticsCollector::new();
        collector.add_error(Text::from("error"), verum_ast::Span::dummy());
        collector.add_warning(Text::from("warning"), verum_ast::Span::dummy());

        let taken = collector.take_diagnostics();
        assert_eq!(taken.len(), 2);
        assert_eq!(collector.error_count(), 0);
        assert_eq!(collector.warning_count(), 0);
        assert!(collector.diagnostics().is_empty());
    }
}
