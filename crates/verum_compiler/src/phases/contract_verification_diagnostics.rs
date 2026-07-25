//! Contract Verification Diagnostics
//!
//! Specialized diagnostic builders for contract verification errors and warnings.
//! Provides clear, actionable error messages with counterexamples and suggestions.

use verum_ast::Span;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_smt::{CounterExample, FailureCategory};

/// Builder for contract verification diagnostics
pub struct ContractVerificationDiagnostic;

impl ContractVerificationDiagnostic {
    /// Create a diagnostic for a failed verification with counterexample
    pub fn verification_failed(
        func_name: &str,
        contract_kind: &str,
        counterexample: &CounterExample,
        span: Span,
    ) -> Diagnostic {
        let message = format!(
            "Contract verification failed for '{}': {} cannot be proven",
            func_name, contract_kind
        );

        DiagnosticBuilder::error()
            .message(message)
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note("Counterexample found:")
            .add_note(format!("{}", counterexample))
            .help("Strengthen the contract or fix the implementation")
            .build()
    }

    /// Create a diagnostic for a verification timeout
    pub fn timeout(
        func_name: &str,
        contract_kind: &str,
        timeout_ms: u64,
        span: Span,
    ) -> Diagnostic {
        DiagnosticBuilder::warning()
            .message(format!(
                "Verification of {} for '{}' timed out after {}ms",
                contract_kind, func_name, timeout_ms
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .help("Consider simplifying the contract or using @verify(runtime)")
            .help("You can increase the timeout with @verify(timeout = N)")
            .add_note("Complex contracts with quantifiers may require longer timeouts")
            .build()
    }

    /// Create a diagnostic for an unsatisfiable precondition
    pub fn unsatisfiable_precondition(func_name: &str, span: Span) -> Diagnostic {
        DiagnosticBuilder::error()
            .message(format!(
                "Precondition for '{}' is unsatisfiable - no valid inputs exist",
                func_name
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .help("Check for contradictory requirements in the precondition")
            .help("Example: 'requires x > 10 && x < 5' is unsatisfiable")
            .build()
    }

    /// Create a diagnostic for a violated postcondition
    pub fn postcondition_violated(
        func_name: &str,
        counterexample: &CounterExample,
        category: FailureCategory,
        span: Span,
    ) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .message(format!("Postcondition for '{}' can be violated", func_name))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note("Counterexample:")
            .add_note(format!("{}", counterexample))
            .add_note(format!("Failure category: {}", category));

        // Add category-specific suggestions
        builder = match category {
            FailureCategory::ArithmeticOverflow => builder
                .help("Check for potential arithmetic overflow")
                .help("Consider using checked arithmetic operations"),
            FailureCategory::DivisionByZero => builder
                .help("Add a precondition to ensure divisor is non-zero")
                .help("Example: requires divisor != 0"),
            FailureCategory::NullDereference => builder
                .help("Ensure all pointer/reference accesses are valid")
                .help("Add preconditions to guarantee non-null values"),
            FailureCategory::IndexOutOfBounds => builder
                .help("Add bounds checking to array/list accesses")
                .help("Example: requires 0 <= index && index < length"),
            FailureCategory::NegativeValue => builder
                .help("Strengthen the precondition to reject invalid input")
                .help("Or adjust the postcondition to handle this case"),
            FailureCategory::Other => {
                builder.help("Review the implementation and contract specifications")
            }
        };

        builder.build()
    }

    /// Create a diagnostic for a type invariant violation
    pub fn type_invariant_violated(
        type_name: &str,
        field_name: Option<&str>,
        counterexample: &CounterExample,
        span: Span,
    ) -> Diagnostic {
        let message = if let Some(field) = field_name {
            format!(
                "Type invariant for field '{}' in type '{}' can be violated",
                field, type_name
            )
        } else {
            format!("Type invariant for type '{}' can be violated", type_name)
        };

        DiagnosticBuilder::error()
            .message(message)
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note("Counterexample:")
            .add_note(format!("{}", counterexample))
            .help("Ensure all type constructors maintain the invariant")
            .help("Check refinement predicates for consistency")
            .build()
    }

    /// Create a diagnostic for unsupported contract features
    pub fn unsupported_feature(func_name: &str, feature: &str, span: Span) -> Diagnostic {
        DiagnosticBuilder::warning()
            .message(format!(
                "Contract for '{}' uses unsupported feature: {}",
                func_name, feature
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .help("This contract will not be verified at compile time")
            .help("Consider using @verify(runtime) for runtime checking")
            .add_note(
                "Supported features: arithmetic, comparisons, boolean logic, basic quantifiers",
            )
            .build()
    }

    /// Create a diagnostic for a contract parsing error
    pub fn parse_error(func_name: &str, error_message: &str, span: Span) -> Diagnostic {
        DiagnosticBuilder::error()
            .message(format!("Failed to parse contract for '{}'", func_name))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note(format!("Parse error: {}", error_message))
            .help("Check contract syntax")
            .help("Contracts use RSL (Refinement Specification Language)")
            .help("Example: requires x > 0, ensures result > x")
            .build()
    }

    /// Create a diagnostic for SMT solver errors
    pub fn smt_error(func_name: &str, error_message: &str, span: Span) -> Diagnostic {
        DiagnosticBuilder::error()
            .message(format!("SMT solver error while verifying '{}'", func_name))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note(format!("Error: {}", error_message))
            .help("This may indicate an issue with contract translation")
            .help("Try simplifying the contract or report this as a bug")
            .build()
    }

    /// Create a diagnostic with suggestions for common failure categories
    pub fn with_suggestions(
        func_name: &str,
        contract_kind: &str,
        category: FailureCategory,
        span: Span,
    ) -> Diagnostic {
        let mut builder = DiagnosticBuilder::warning()
            .message(format!(
                "Potential issue in {} for '{}'",
                contract_kind, func_name
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note(format!("Issue category: {}", category));

        builder = match category {
            FailureCategory::ArithmeticOverflow => builder
                .help("Use checked arithmetic: checked_add, checked_mul, etc.")
                .help("Add preconditions to limit input ranges")
                .help("Example: requires x <= Int::MAX / 2"),
            FailureCategory::DivisionByZero => builder
                .help("Add precondition: requires divisor != 0")
                .help("Or handle zero case separately in the implementation"),
            FailureCategory::NullDereference => builder
                .help("Use Maybe/Option types for nullable values")
                .help("Add preconditions to guarantee non-null"),
            FailureCategory::IndexOutOfBounds => builder
                .help("Add bounds checking precondition")
                .help("Example: requires 0 <= i && i < list.length")
                .help("Use safe indexing methods like get() instead of []"),
            FailureCategory::NegativeValue => builder
                .help("Strengthen preconditions to reject invalid inputs")
                .help("Document valid input ranges in contracts"),
            FailureCategory::Other => {
                builder.help("Review contract specifications and implementation")
            }
        };

        builder.build()
    }

    /// Create an informational diagnostic for successful verification
    pub fn verification_success(
        func_name: &str,
        contract_count: usize,
        duration_ms: u64,
        span: Span,
    ) -> Diagnostic {
        DiagnosticBuilder::new(Severity::Note)
            .message(format!(
                "Successfully verified {} contract(s) for '{}' in {}ms",
                contract_count, func_name, duration_ms
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .build()
    }

    /// Create a warning for contracts that can't be fully verified
    pub fn partial_verification(
        func_name: &str,
        verified_count: usize,
        total_count: usize,
        span: Span,
    ) -> Diagnostic {
        DiagnosticBuilder::warning()
            .message(format!(
                "Partial verification for '{}': {}/{} contracts verified",
                func_name, verified_count, total_count
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .help("Some contracts could not be verified due to timeouts or complexity")
            .help("Consider using @verify(runtime) for unverified contracts")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_diagnostic() {
        let diag = ContractVerificationDiagnostic::timeout(
            "factorial",
            "postcondition",
            5000,
            Span::dummy(),
        );

        assert_eq!(diag.severity(), Severity::Warning);
        assert!(diag.message().contains("timed out"));
        assert!(diag.message().contains("5000ms"));
    }

    #[test]
    fn test_unsatisfiable_precondition_diagnostic() {
        let diag =
            ContractVerificationDiagnostic::unsatisfiable_precondition("test_func", Span::dummy());

        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("unsatisfiable"));
        assert!(diag.message().contains("no valid inputs"));
    }

    #[test]
    fn test_parse_error_diagnostic() {
        let diag = ContractVerificationDiagnostic::parse_error(
            "my_func",
            "unexpected token",
            Span::dummy(),
        );

        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("Failed to parse"));
    }

    #[test]
    fn test_verification_success_diagnostic() {
        let diag = ContractVerificationDiagnostic::verification_success(
            "safe_divide",
            2,
            150,
            Span::dummy(),
        );

        assert_eq!(diag.severity(), Severity::Note);
        assert!(diag.message().contains("Successfully verified"));
        assert!(diag.message().contains("2 contract"));
    }
}
