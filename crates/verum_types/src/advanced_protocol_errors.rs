//! Enhanced Error Messages for GATs and Specialization
//!
//! Higher-kinded type (HKT) kind inference: infers kinds for type constructors
//! (e.g., List has kind Type -> Type, Map has kind Type -> Type -> Type).
//! Uses constraint-based kind inference with unification.
//!
//! This module provides detailed, actionable error messages for advanced protocol features:
//! - GAT arity mismatches with suggestions
//! - Specialization ambiguities with candidate listings
//! - GenRef generation mismatches (use-after-free detection)
//! - GAT where clause violations
//! - Negative specialization errors
//!
//! # Design Philosophy
//!
//! All error messages follow these principles:
//! 1. **Clear context**: What went wrong and where
//! 2. **Actual vs Expected**: Show what was found vs what was needed
//! 3. **Actionable suggestions**: Provide fix recommendations
//! 4. **Code examples**: Show correct usage patterns
//! 5. **Rich formatting**: Use labels, notes, and help messages
//!
//! # Integration
//!
//! These error types integrate with verum_diagnostics for rich terminal output
//! with colors, labels, and multi-line highlighting.

use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path};
use verum_common::span::FileId;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Label as DiagLabel, SpanLabel};
use verum_common::{List, Map, Text};
use verum_common::ToText;

use crate::protocol::{ProtocolBound, WhereClause};
use crate::ty::{Type, TypeVar};

// ==================== GAT Arity Mismatch Errors ====================

/// GAT arity mismatch error with rich diagnostics
///
/// Spec: Section 9.1.1
///
/// Example:
/// ```verum
/// protocol Iterator {
///     type Item<T>  // Expects 1 type parameter
/// }
///
/// fn foo() -> Maybe<Item> { ... }  // ERROR: Missing type argument
/// ```
#[derive(Debug, Clone)]
pub struct GATArityError {
    /// GAT name (e.g., "Item")
    pub gat_name: Text,
    /// Expected number of type parameters
    pub expected_arity: usize,
    /// Found number of type parameters
    pub found_arity: usize,
    /// Span where the error occurred
    pub span: Span,
    /// Protocol name for context
    pub protocol_name: Option<Text>,
    /// Expected type parameter names for better suggestions
    pub expected_params: Option<List<Text>>,
}

impl GATArityError {
    /// Create a new GAT arity error
    pub fn new(
        gat_name: impl Into<Text>,
        expected_arity: usize,
        found_arity: usize,
        span: Span,
    ) -> Self {
        Self {
            gat_name: gat_name.into(),
            expected_arity,
            found_arity,
            span,
            protocol_name: None,
            expected_params: None,
        }
    }

    /// Set the protocol name for better context
    pub fn with_protocol(mut self, protocol_name: impl Into<Text>) -> Self {
        self.protocol_name = Some(protocol_name.into());
        self
    }

    /// Set expected parameter names for better suggestions
    pub fn with_expected_params(mut self, params: List<Text>) -> Self {
        self.expected_params = Some(params);
        self
    }

    /// Generate incorrect usage example
    fn incorrect_usage(&self) -> Text {
        if self.found_arity == 0 {
            self.gat_name.clone()
        } else {
            Text::from(format!(
                "{}<{}>",
                self.gat_name,
                (0..self.found_arity)
                    .map(|i| format!("T{}", i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }

    /// Generate correct usage example
    pub fn correct_usage(&self) -> Text {
        if self.expected_arity == 0 {
            self.gat_name.clone()
        } else if let Some(ref params) = self.expected_params {
            Text::from(format!(
                "{}<{}>",
                self.gat_name,
                params
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        } else {
            Text::from(format!(
                "{}<{}>",
                self.gat_name,
                (0..self.expected_arity)
                    .map(|i| format!("T{}", i))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    }

    /// Convert to diagnostic with rich formatting
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error().code("E0307").message(format!(
            "GAT `{}` expects {} type parameter(s), but {} were provided",
            self.gat_name, self.expected_arity, self.found_arity
        ));

        // Add primary label
        let span_label = SpanLabel::primary(
            span_to_line_col(self.span),
            format!("expected {} type argument(s)", self.expected_arity),
        );
        builder = builder.span_label(span_to_line_col(self.span), span_label.message.clone());

        // Add context note
        let context_msg = if let Some(ref protocol) = self.protocol_name {
            format!(
                "Note: `{}` is defined in protocol `{}` with {} type parameter(s)",
                self.gat_name, protocol, self.expected_arity
            )
        } else {
            format!(
                "Note: `{}` is defined with {} type parameter(s)",
                self.gat_name, self.expected_arity
            )
        };
        builder = builder.add_note(context_msg);

        // Add help message based on whether we need to add or remove parameters
        if self.found_arity < self.expected_arity {
            let missing = self.expected_arity - self.found_arity;
            builder = builder.help(format!(
                "Add {} more type argument(s) to match the GAT definition",
                missing
            ));
        } else {
            let extra = self.found_arity - self.expected_arity;
            builder = builder.help(format!(
                "Remove {} type argument(s) to match the GAT definition",
                extra
            ));
        }

        // Add code example
        builder = builder.help(format!(
            "Change `{}` to `{}`",
            self.incorrect_usage(),
            self.correct_usage()
        ));

        builder.build()
    }
}

// ==================== Specialization Ambiguity Errors ====================

/// Implementation ID for tracking protocol implementations
pub type ImplementId = usize;

/// Specialization ambiguity error
///
/// Spec: Section 9.1.2
///
/// Example:
/// ```verum
/// implement<T: Clone> Display for List<T> { ... }
/// implement<T: Send> Display for List<T> { ... }
///
/// // ERROR: Both implementations are equally specific for List<SomeType>
/// ```
#[derive(Debug, Clone)]
pub struct SpecializationAmbiguityError {
    /// Protocol being implemented
    pub protocol: Text,
    /// Self type being implemented for
    pub self_type: Type,
    /// Candidate implementation IDs
    pub candidates: List<ImplementId>,
    /// Span where the error occurred
    pub span: Span,
    /// Details about each candidate (optional, for richer output)
    pub candidate_details: Option<List<CandidateInfo>>,
}

/// Information about a specialization candidate
#[derive(Debug, Clone)]
pub struct CandidateInfo {
    /// Implementation ID
    pub impl_id: ImplementId,
    /// Source location of the implementation
    pub span: Option<Span>,
    /// String representation of the implementation signature
    pub signature: Text,
    /// Bounds on the implementation
    pub bounds: List<Text>,
}

impl SpecializationAmbiguityError {
    /// Create a new specialization ambiguity error
    pub fn new(
        protocol: impl Into<Text>,
        self_type: Type,
        candidates: List<ImplementId>,
        span: Span,
    ) -> Self {
        Self {
            protocol: protocol.into(),
            self_type,
            candidates,
            span,
            candidate_details: None,
        }
    }

    /// Add candidate details for richer error messages
    pub fn with_candidate_details(mut self, details: List<CandidateInfo>) -> Self {
        self.candidate_details = Some(details);
        self
    }

    /// Convert to diagnostic with rich formatting
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error().code("E0308").message(format!(
            "Ambiguous specialization for `{} for {}`",
            self.protocol,
            format_type(&self.self_type)
        ));

        // Add primary label
        builder = builder.span_label(
            span_to_line_col(self.span),
            "multiple equally specific implementations",
        );

        // Add note about candidates
        builder = builder.add_note(format!(
            "Found {} equally specific implementations:",
            self.candidates.len()
        ));

        // Add details for each candidate if available
        if let Some(ref details) = self.candidate_details {
            for (i, detail) in details.iter().enumerate() {
                // Add secondary label if we have span info
                if let Some(ref candidate_span) = detail.span {
                    builder = builder.secondary_span(
                        span_to_line_col(*candidate_span),
                        format!("candidate #{}: {}", i + 1, detail.signature),
                    );
                }

                // Add note with bounds info
                if !detail.bounds.is_empty() {
                    builder = builder.add_note(format!(
                        "  Candidate #{}: {} (where {})",
                        i + 1,
                        detail.signature,
                        detail
                            .bounds
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                } else {
                    builder =
                        builder.add_note(format!("  Candidate #{}: {}", i + 1, detail.signature));
                }
            }
        } else {
            // Fallback if we don't have detailed info
            for (i, candidate_id) in self.candidates.iter().enumerate() {
                builder =
                    builder.add_note(format!("  Candidate #{} (impl #{})", i + 1, candidate_id));
            }
        }

        // Add help message with specialization ranking
        builder = builder.add_note("");
        builder = builder.help("Add @specialize attribute to specify precedence:");
        builder = builder.help("  @specialize(rank = 10)  // Higher rank = higher priority");
        builder = builder.help("  implement<T: Clone> Display for List<T> { ... }");

        builder.build()
    }
}

// ==================== GenRef Generation Mismatch Errors ====================

/// GenRef generation mismatch error (use-after-free detection)
///
/// Spec: Section 9.1.3
///
/// Example:
/// ```verum
/// let genref = GenRef::new(value);
/// free(value);  // Object freed
/// *genref       // ERROR: GenRef generation mismatch
/// ```
#[derive(Debug, Clone)]
pub struct GenerationMismatchError {
    /// Expected generation counter
    pub expected_gen: u64,
    /// Found generation counter
    pub found_gen: u64,
    /// Pointer address for debugging
    pub ptr: usize,
    /// Span where the error occurred
    pub span: Span,
    /// Optional context about what operation triggered the error
    pub operation: Option<Text>,
}

impl GenerationMismatchError {
    /// Create a new generation mismatch error
    pub fn new(expected_gen: u64, found_gen: u64, ptr: usize, span: Span) -> Self {
        Self {
            expected_gen,
            found_gen,
            ptr,
            span,
            operation: None,
        }
    }

    /// Set the operation that triggered the error
    pub fn with_operation(mut self, operation: impl Into<Text>) -> Self {
        self.operation = Some(operation.into());
        self
    }

    /// Convert to diagnostic with rich formatting
    pub fn to_diagnostic(&self) -> Diagnostic {
        let op_msg = if let Some(ref op) = self.operation {
            format!(" during {}", op)
        } else {
            String::new()
        };

        let mut builder = DiagnosticBuilder::error().code("E0309").message(format!(
            "GenRef generation mismatch (use-after-free detected){}",
            op_msg
        ));

        // Add primary label
        builder = builder.span_label(span_to_line_col(self.span), "dereferencing stale GenRef");

        // Add detailed notes
        builder = builder.add_note(format!("Expected generation: {}", self.expected_gen));
        builder = builder.add_note(format!("Found generation: {}", self.found_gen));
        builder = builder.add_note(format!("Pointer: 0x{:x}", self.ptr));
        builder = builder.add_note("");
        builder = builder.add_note("This indicates the object was freed and reallocated.");
        builder = builder.add_note("");

        // Add help messages
        builder = builder.help("Ensure the GenRef is still valid before dereferencing:");
        builder = builder.help("  if genref.is_valid() {");
        builder = builder.help("      // Safe to dereference");
        builder = builder.help("      *genref");
        builder = builder.help("  }");

        builder.build()
    }
}

// ==================== GAT Where Clause Violation Errors ====================

/// GAT where clause violation error
///
/// Spec: Section 9.1.4
///
/// Example:
/// ```verum
/// protocol Collection {
///     type Item<K, V> where K: Hash
/// }
///
/// let x: Collection::Item<Text, Int> = ...;  // ERROR: Text doesn't implement Hash
/// ```
#[derive(Debug, Clone)]
pub struct GATWhereClauseError {
    /// GAT name
    pub gat_name: Text,
    /// The where clause that wasn't satisfied
    pub clause: WhereClause,
    /// Type instantiation that violated the clause
    pub instantiation: Map<Text, Type>,
    /// Span where the error occurred
    pub span: Span,
    /// Protocol name for context
    pub protocol_name: Option<Text>,
}

impl GATWhereClauseError {
    /// Create a new GAT where clause error
    pub fn new(
        gat_name: impl Into<Text>,
        clause: WhereClause,
        instantiation: Map<Text, Type>,
        span: Span,
    ) -> Self {
        Self {
            gat_name: gat_name.into(),
            clause,
            instantiation,
            span,
            protocol_name: None,
        }
    }

    /// Set the protocol name for better context
    pub fn with_protocol(mut self, protocol_name: impl Into<Text>) -> Self {
        self.protocol_name = Some(protocol_name.into());
        self
    }

    /// Format the type instantiation for display
    fn format_instantiation(&self) -> Text {
        let entries: Vec<String> = self
            .instantiation
            .iter()
            .map(|(k, v)| format!("{} = {}", k, format_type(v)))
            .collect();
        Text::from(entries.join(", "))
    }

    /// Convert to diagnostic with rich formatting
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error().code("E0310").message(format!(
            "GAT `{}` where clause not satisfied",
            self.gat_name
        ));

        // Add primary label with constraint info
        builder = builder.span_label(
            span_to_line_col(self.span),
            format!("constraint `{}` not met", format_where_clause(&self.clause)),
        );

        // Add context notes
        if let Some(ref protocol) = self.protocol_name {
            builder = builder.add_note(format!(
                "GAT `{}` in protocol `{}` requires:",
                self.gat_name, protocol
            ));
        } else {
            builder = builder.add_note("Where clause requires:");
        }

        builder = builder.add_note(format!("  {}", format_where_clause(&self.clause)));
        builder = builder.add_note("");
        builder = builder.add_note("But the type instantiation:");
        builder = builder.add_note(format!("  {}", self.format_instantiation()));
        builder = builder.add_note("");
        builder = builder.add_note("does not satisfy this constraint.");

        // Add help message with suggestion
        if let Some(missing_impl) = self.get_missing_implementation() {
            builder = builder.add_note("");
            builder = builder.help(format!(
                "Consider implementing `{}` for the type",
                missing_impl
            ));
        } else {
            // Fallback help message when we can't determine specific missing implementation
            builder = builder
                .help("Ensure the type instantiation satisfies all where clause constraints");
        }

        builder.build()
    }

    /// Try to extract the missing protocol implementation from the where clause
    ///
    /// Production implementation that analyzes the where clause structure to provide
    /// actionable suggestions for fixing GAT constraint violations.
    fn get_missing_implementation(&self) -> Option<Text> {
        // Check if there are actual bounds to satisfy
        if self.clause.bounds.is_empty() {
            return None;
        }

        // Build a detailed suggestion based on the constraint structure
        let mut suggestions = List::new();

        for bound in &self.clause.bounds {
            // Extract the protocol name from the bound
            let protocol_name = format_protocol_bound(bound);

            // Determine the type that needs to implement the protocol
            let type_name = format_type(&self.clause.ty);

            // Try to substitute actual types from instantiation into the type
            let concrete_type = self.substitute_type_params(&self.clause.ty);

            // Build the implementation suggestion
            if concrete_type == type_name {
                suggestions.push(format!("implement {} for {}", protocol_name, type_name));
            } else {
                suggestions.push(format!(
                    "implement {} for {} (instantiated as {})",
                    protocol_name, type_name, concrete_type
                ));
            }
        }

        if suggestions.is_empty() {
            None
        } else if suggestions.len() == 1 {
            Some(Text::from(suggestions.into_iter().next().unwrap()))
        } else {
            // Multiple bounds - format as a list
            Some(suggestions.join(", or "))
        }
    }

    /// Substitute type parameters from the instantiation map into a type
    fn substitute_type_params(&self, ty: &Type) -> Text {
        let type_str = format_type(ty);

        // Check if this type is a simple type variable we have an instantiation for
        if let Type::Var(var) = ty {
            let var_key = var.to_text();
            if let Some(concrete) = self.instantiation.get(&var_key) {
                return format_type(concrete);
            }
        }

        // For named types, recursively substitute arguments
        if let Type::Named { path, args } = ty {
            if args.is_empty() {
                return type_str;
            }

            // Build substituted args
            let substituted_args: Vec<String> = args
                .iter()
                .map(|arg| self.substitute_type_params(arg).to_string())
                .collect();

            // Format path
            let path_str: String = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    verum_ast::ty::PathSegment::Name(ident) => {
                        Some(ident.name.as_str().to_string())
                    }
                    verum_ast::ty::PathSegment::SelfValue => Some("Self".to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("::");

            return Text::from(format!("{}<{}>", path_str, substituted_args.join(", ")));
        }

        type_str
    }
}

// ==================== Negative Specialization Errors ====================

/// Negative specialization error
///
/// Spec: Section 9.1.5
///
/// Example:
/// ```verum
/// @specialize(negative)
/// implement<T: !Clone> Default for Wrapper<T> { ... }
///
/// // ERROR: T implements Clone, violating the negative bound
/// ```
#[derive(Debug, Clone)]
pub struct NegativeSpecializationError {
    /// Protocol being specialized
    pub protocol: Text,
    /// Self type
    pub self_type: Type,
    /// The negative bound that was violated
    pub negative_bound: ProtocolBound,
    /// Span where the error occurred
    pub span: Span,
    /// Why the type satisfies the negative bound (shouldn't)
    pub reason: Option<Text>,
}

impl NegativeSpecializationError {
    /// Create a new negative specialization error
    pub fn new(
        protocol: impl Into<Text>,
        self_type: Type,
        negative_bound: ProtocolBound,
        span: Span,
    ) -> Self {
        Self {
            protocol: protocol.into(),
            self_type,
            negative_bound,
            span,
            reason: None,
        }
    }

    /// Set the reason why the negative bound was violated
    pub fn with_reason(mut self, reason: impl Into<Text>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Convert to diagnostic with rich formatting
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error().code("E0311").message(format!(
            "Negative specialization requires `!{}`",
            format_protocol_bound(&self.negative_bound)
        ));

        // Add primary label
        let label_msg = format!(
            "but `{}` implements `{}`",
            format_type(&self.self_type),
            format_protocol_bound(&self.negative_bound)
        );
        builder = builder.span_label(span_to_line_col(self.span), label_msg);

        // Add explanation
        builder = builder.add_note("Negative specialization (@specialize(negative)) requires");
        builder = builder.add_note(format!(
            "that the type does NOT implement `{}`",
            format_protocol_bound(&self.negative_bound)
        ));

        // Add reason if provided
        if let Some(ref reason) = self.reason {
            builder = builder.add_note("");
            builder = builder.add_note(format!("Reason: {}", reason));
        }

        // Add help
        builder = builder.add_note("");
        builder = builder.help("Remove the protocol implementation, or");
        builder = builder.help("Use a positive specialization instead:");
        builder = builder.help(format!(
            "  @specialize  // Positive specialization for types that DO implement {}",
            format_protocol_bound(&self.negative_bound)
        ));

        builder.build()
    }
}

// ==================== Helper Functions ====================

/// Format a type for error messages using the Type's Display implementation
fn format_type(ty: &Type) -> Text {
    // Use Display trait for human-readable output
    Text::from(format!("{}", ty))
}

/// Format a protocol bound for error messages
fn format_protocol_bound(bound: &ProtocolBound) -> Text {
    use verum_ast::ty::PathSegment;

    // Format the protocol path as a simple string (e.g., "Clone", "Send")
    let path_str = bound
        .protocol
        .segments
        .iter()
        .filter_map(|seg| match seg {
            PathSegment::Name(ident) => Some(ident.name.as_str()),
            PathSegment::SelfValue => Some("Self"),
            PathSegment::Super => Some("super"),
            PathSegment::Cog => Some("cog"),
            PathSegment::Relative => None,
        })
        .collect::<Vec<_>>()
        .join("::");

    if bound.args.is_empty() {
        Text::from(path_str)
    } else {
        // Format with generic arguments using Display trait for human-readable output
        let args_str: Vec<String> = bound.args.iter().map(|arg| format!("{}", arg)).collect();
        Text::from(format!("{}<{}>", path_str, args_str.join(", ")))
    }
}

/// Format a where clause for error messages
fn format_where_clause(clause: &WhereClause) -> Text {
    // Format as "Type: Bound1 + Bound2" for human-readable output
    let bounds_str: Vec<String> = clause
        .bounds
        .iter()
        .map(|b| format_protocol_bound(b).to_string())
        .collect();
    if bounds_str.is_empty() {
        format_type(&clause.ty)
    } else {
        Text::from(format!("{}: {}", clause.ty, bounds_str.join(" + ")))
    }
}

// Helper function to convert Span to LineColSpan using the global source file registry
fn span_to_line_col(span: Span) -> verum_common::span::LineColSpan {
    // Use the global source file registry for proper file mapping
    crate::source_files::span_to_line_col(span)
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_span() -> Span {
        Span::new(0, 10, FileId::new(0))
    }

    #[test]
    fn test_gat_arity_error_formatting() {
        let error = GATArityError::new("Item", 1, 0, dummy_span())
            .with_protocol("Iterator")
            .with_expected_params(List::from(vec!["T".into()]));

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        assert_eq!(diag.code(), Some("E0307"));
        assert!(diag.message().contains("Item"));
        assert!(diag.message().contains("1 type parameter"));
    }

    #[test]
    fn test_gat_arity_error_zero_to_one() {
        let error = GATArityError::new("Output", 1, 0, dummy_span());
        assert_eq!(error.incorrect_usage(), "Output");
        assert_eq!(error.correct_usage(), "Output<T0>");
    }

    #[test]
    fn test_gat_arity_error_one_to_two() {
        let error = GATArityError::new("Result", 2, 1, dummy_span())
            .with_expected_params(List::from(vec!["T".into(), "E".into()]));
        assert_eq!(error.correct_usage(), "Result<T, E>");
    }

    #[test]
    fn test_specialization_ambiguity_message() {
        let error = SpecializationAmbiguityError::new(
            "Display",
            Type::Named {
                path: Path::single(Ident::new("List<Int>", Span::default())),
                args: List::new(),
            },
            List::from(vec![1, 2]),
            dummy_span(),
        );

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        assert_eq!(diag.code(), Some("E0308"));
        assert!(diag.message().contains("Ambiguous specialization"));
        assert!(diag.message().contains("Display"));
    }

    #[test]
    fn test_specialization_ambiguity_with_details() {
        let details = List::from(vec![
            CandidateInfo {
                impl_id: 1,
                span: None,
                signature: "implement<T: Clone> Display for List<T>".into(),
                bounds: List::from(vec!["T: Clone".into()]),
            },
            CandidateInfo {
                impl_id: 2,
                span: None,
                signature: "implement<T: Send> Display for List<T>".into(),
                bounds: List::from(vec!["T: Send".into()]),
            },
        ]);

        let error = SpecializationAmbiguityError::new(
            "Display",
            Type::Named {
                path: Path::single(Ident::new("List<Int>", Span::default())),
                args: List::new(),
            },
            List::from(vec![1, 2]),
            dummy_span(),
        )
        .with_candidate_details(details);

        let diag = error.to_diagnostic();
        assert!(!diag.notes().is_empty());
    }

    #[test]
    fn test_genref_generation_mismatch() {
        let error = GenerationMismatchError::new(42, 43, 0x7ffee4c0a000, dummy_span())
            .with_operation("dereference");

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        assert_eq!(diag.code(), Some("E0309"));
        assert!(diag.message().contains("generation mismatch"));
        assert!(diag.message().contains("dereference"));
    }

    #[test]
    fn test_genref_generation_values() {
        let error = GenerationMismatchError::new(100, 105, 0x1234, dummy_span());
        let diag = error.to_diagnostic();

        let notes = diag.notes();
        let note_text: String = notes.iter().map(|n| n.to_string()).collect();
        assert!(note_text.contains("100"));
        assert!(note_text.contains("105"));
    }

    #[test]
    fn test_where_clause_violation() {
        let clause = WhereClause {
            ty: Type::Named {
                path: Path::single(Ident::new("K", Span::default())),
                args: List::new(),
            },
            bounds: List::from(vec![ProtocolBound {
                protocol: Path::single(Ident::new("Hash", Span::default())),
                args: List::new(),
                is_negative: false,
            }]),
        };

        let mut instantiation = Map::new();
        instantiation.insert(
            "K".into(),
            Type::Named {
                path: Path::single(Ident::new("Text", Span::default())),
                args: List::new(),
            },
        );
        instantiation.insert(
            "V".into(),
            Type::Named {
                path: Path::single(Ident::new("Int", Span::default())),
                args: List::new(),
            },
        );

        let error = GATWhereClauseError::new("Item", clause, instantiation, dummy_span())
            .with_protocol("Collection");

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        assert_eq!(diag.code(), Some("E0310"));
        assert!(diag.message().contains("where clause not satisfied"));
    }

    #[test]
    fn test_where_clause_instantiation_format() {
        let clause = WhereClause {
            ty: Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            bounds: List::new(),
        };

        let mut instantiation = Map::new();
        instantiation.insert(
            "K".into(),
            Type::Named {
                path: Path::single(Ident::new("Text", Span::default())),
                args: List::new(),
            },
        );
        instantiation.insert(
            "V".into(),
            Type::Named {
                path: Path::single(Ident::new("Int", Span::default())),
                args: List::new(),
            },
        );

        let error = GATWhereClauseError::new("Item", clause, instantiation, dummy_span());
        let formatted = error.format_instantiation();

        // Should contain both K and V mappings
        assert!(formatted.contains("K =") || formatted.contains("V ="));
    }

    #[test]
    fn test_negative_specialization_error() {
        let bound = ProtocolBound {
            protocol: Path::single(Ident::new("Clone", Span::default())),
            args: List::new(),
            is_negative: true,
        };

        // Create Wrapper<MyType> using proper type construction
        let my_type = Type::Named {
            path: Path::single(Ident::new("MyType", Span::default())),
            args: List::new(),
        };
        let wrapper_type = Type::Named {
            path: Path::single(Ident::new("Wrapper", Span::default())),
            args: vec![my_type].into(),
        };

        let error = NegativeSpecializationError::new("Default", wrapper_type, bound, dummy_span())
            .with_reason("MyType implements Clone");

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        assert_eq!(diag.code(), Some("E0311"));
        assert!(diag.message().contains("Negative specialization"));
        assert!(diag.message().contains("!Clone"));
    }

    #[test]
    fn test_negative_specialization_without_reason() {
        let bound = ProtocolBound {
            protocol: Path::single(Ident::new("Send", Span::default())),
            args: List::new(),
            is_negative: true,
        };

        let error = NegativeSpecializationError::new(
            "MyProtocol",
            Type::Named {
                path: Path::single(Ident::new("MyType", Span::default())),
                args: List::new(),
            },
            bound,
            dummy_span(),
        );

        let diag = error.to_diagnostic();
        assert!(diag.is_error());
        // Should still work without a reason
    }

    #[test]
    fn test_error_codes_are_unique() {
        let gat_error = GATArityError::new("T", 1, 0, dummy_span());
        let spec_error = SpecializationAmbiguityError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            List::new(),
            dummy_span(),
        );
        let gen_error = GenerationMismatchError::new(1, 2, 0, dummy_span());
        let where_error = GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: Type::Named {
                    path: Path::single(Ident::new("T", Span::default())),
                    args: List::new(),
                },
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        );
        let neg_error = NegativeSpecializationError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            ProtocolBound {
                protocol: Path::single(Ident::new("P", Span::default())),
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        );

        // Each error should have a unique code
        let gat_diag = gat_error.to_diagnostic();
        let spec_diag = spec_error.to_diagnostic();
        let gen_diag = gen_error.to_diagnostic();
        let where_diag = where_error.to_diagnostic();
        let neg_diag = neg_error.to_diagnostic();

        let codes = [
            gat_diag.code(),
            spec_diag.code(),
            gen_diag.code(),
            where_diag.code(),
            neg_diag.code(),
        ];

        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j], "Error codes must be unique");
            }
        }
    }

    #[test]
    fn test_all_errors_have_codes() {
        let gat_error = GATArityError::new("T", 1, 0, dummy_span());
        let spec_error = SpecializationAmbiguityError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            List::new(),
            dummy_span(),
        );
        let gen_error = GenerationMismatchError::new(1, 2, 0, dummy_span());
        let where_error = GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: Type::Named {
                    path: Path::single(Ident::new("T", Span::default())),
                    args: List::new(),
                },
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        );
        let neg_error = NegativeSpecializationError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            ProtocolBound {
                protocol: Path::single(Ident::new("P", Span::default())),
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        );

        assert!(gat_error.to_diagnostic().code().is_some());
        assert!(spec_error.to_diagnostic().code().is_some());
        assert!(gen_error.to_diagnostic().code().is_some());
        assert!(where_error.to_diagnostic().code().is_some());
        assert!(neg_error.to_diagnostic().code().is_some());
    }

    #[test]
    fn test_all_errors_have_help_messages() {
        let gat_error = GATArityError::new("T", 1, 0, dummy_span());
        let spec_error = SpecializationAmbiguityError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            List::from(vec![1]),
            dummy_span(),
        );
        let gen_error = GenerationMismatchError::new(1, 2, 0, dummy_span());
        let where_error = GATWhereClauseError::new(
            "T",
            WhereClause {
                ty: Type::Named {
                    path: Path::single(Ident::new("T", Span::default())),
                    args: List::new(),
                },
                bounds: List::new(),
            },
            Map::new(),
            dummy_span(),
        );
        let neg_error = NegativeSpecializationError::new(
            "P",
            Type::Named {
                path: Path::single(Ident::new("T", Span::default())),
                args: List::new(),
            },
            ProtocolBound {
                protocol: Path::single(Ident::new("P", Span::default())),
                args: List::new(),
                is_negative: true,
            },
            dummy_span(),
        );

        // All diagnostics should have help messages
        assert!(!gat_error.to_diagnostic().helps().is_empty());
        assert!(!spec_error.to_diagnostic().helps().is_empty());
        assert!(!gen_error.to_diagnostic().helps().is_empty());
        assert!(!where_error.to_diagnostic().helps().is_empty());
        assert!(!neg_error.to_diagnostic().helps().is_empty());
    }
}
