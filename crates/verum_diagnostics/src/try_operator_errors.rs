//! Specialized diagnostics for the `?` operator (try operator).
//!
//! This module implements error codes E0203, E0204, and E0205 for the '?' (try) operator.
//!
//! The '?' operator desugars to: match expr { Ok(v) => v, Err(e) => return Err(e.into()) }.
//! It requires: (1) the enclosing function returns Result<T, E> or Maybe<T>, and
//! (2) there is a From<InnerError> for OuterError implementation when error types differ.
//! E0203 fires on incompatible error types, E0204 on ambiguous multiple conversion paths,
//! and E0205 when '?' is used in a function that doesn't return Result/Maybe.
//!
//! The `?` operator provides ergonomic error propagation, but requires careful
//! type checking to ensure errors are properly converted and propagated.
//!
//! # Error Codes
//!
//! - **E0203**: Result type mismatch - error types not compatible
//! - **E0204**: Missing From implementation - no conversion path exists
//! - **E0205**: Cannot use `?` in non-Result context - function doesn't return Result
//!
//! # Design Philosophy
//!
//! These diagnostics focus on providing:
//! 1. **Clear problem identification** - What exactly went wrong
//! 2. **Concrete examples** - Show code before and after fixes
//! 3. **Multiple fix suggestions** - 3-4 actionable options ranked by best practice
//! 4. **Documentation links** - Point to relevant specs and guides

use crate::{Diagnostic, DiagnosticBuilder, Severity, Span};
use verum_common::{List, Text};

/// Error code for Result type mismatch in `?` operator
pub const E0203: &str = "E0203";

/// Error code for missing From implementation
pub const E0204: &str = "E0204";

/// Error code for using `?` in non-Result context
pub const E0205: &str = "E0205";

/// Creates a diagnostic for E0203: Result type mismatch
///
/// This error occurs when a function returns Result<T, E1>, but the `?` operator
/// tries to propagate Result<U, E2> where E1 and E2 are not compatible.
///
/// # Parameters
///
/// - `span`: The location of the `?` operator
/// - `inner_error_type`: The error type from the inner Result (E2)
/// - `outer_error_type`: The expected error type from the function signature (E1)
/// - `expr_span`: The span of the expression being propagated
/// - `function_return_span`: The span of the function's return type declaration
///
/// # Example
///
/// ```verum
/// fn process_config() -> Result<Config, AppError> {
///     let content = read_file("config.txt")?;  // Returns Result<Text, IoError>
///     //                                     ^ E0203: cannot convert IoError to AppError
///     parse_config(&content)
/// }
/// ```
pub fn e0203_result_type_mismatch(
    span: Span,
    inner_error_type: &Text,
    outer_error_type: &Text,
    expr_span: Span,
    function_return_span: Option<Span>,
) -> Diagnostic {
    let mut builder = DiagnosticBuilder::error()
        .code(E0203)
        .message(format!(
            "type mismatch in '?' operator: cannot convert {} to {}",
            inner_error_type, outer_error_type
        ))
        .span_label(
            span,
            format!(
                "cannot convert {} to {}",
                inner_error_type, outer_error_type
            ),
        )
        .secondary_span(
            expr_span,
            format!("this expression has error type {}", inner_error_type),
        );

    // Add secondary label for function return type if available
    if let Some(ret_span) = function_return_span {
        builder = builder.secondary_span(
            ret_span,
            format!("function returns Result<_, {}>", outer_error_type),
        );
    }

    builder
        .add_note(format!(
            "The '?' operator requires that {} can be converted to {}",
            inner_error_type, outer_error_type
        ))
        .add_note(format!(
            "This requires a From<{}> for {} implementation",
            inner_error_type, outer_error_type
        ))
        .help(format!(
            "Add a From implementation to enable automatic conversion:\n\
            \n\
            implement From<{}> for {} {{\n\
                fn from(err: {}) -> Self {{\n\
                    // Convert {} to {}\n\
                    Self::Io(err)  // Wrap in variant\n\
                }}\n\
            }}",
            inner_error_type,
            outer_error_type,
            inner_error_type,
            inner_error_type,
            outer_error_type
        ))
        .help(format!(
            "Use map_err to explicitly convert the error:\n\
            \n\
            let content = read_file(\"config.txt\")\n\
                .map_err({}::from)?;",
            outer_error_type
        ))
        .help(format!(
            "Match and handle the error explicitly:\n\
            \n\
            let content = match read_file(\"config.txt\") {{\n\
                Ok(content) => content,\n\
                Err(e) => return Err({}::from(e)),\n\
            }};",
            outer_error_type
        ))
        .help(format!(
            "Use universal error conversion pattern (if any error implements Error protocol):\n\
            \n\
            implement<E: Error> From<E> for {} {{\n\
                fn from(err: E) -> Self {{\n\
                    Self::Generic(err.to_string())\n\
                }}\n\
            }}",
            outer_error_type
        ))
        .add_note("The '?' operator desugars to match/Err(e.into()); a From<SourceError> for TargetError implementation is required for automatic error conversion")
        .build()
}

/// Creates a diagnostic for E0204: Multiple conversion paths detected
///
/// This error occurs when there are multiple ways to convert from one error type
/// to another, creating ambiguity in which path the `?` operator should use.
///
/// # Parameters
///
/// - `span`: The location of the `?` operator
/// - `from_type`: The source error type
/// - `to_type`: The target error type
/// - `paths`: List of conversion path descriptions
///
/// # Example
///
/// ```verum
/// // Multiple conversion paths:
/// implement From<ErrorA> for AppError { /* direct */ }
/// implement From<ErrorA> for ErrorB { /* indirect */ }
/// implement From<ErrorB> for AppError { /* indirect */ }
///
/// fn process() -> Result<Data, AppError> {
///     operation_a()?;  // E0204: ambiguous - ErrorA -> AppError (direct or via ErrorB?)
/// }
/// ```
pub fn e0204_multiple_conversion_paths(
    span: Span,
    from_type: &Text,
    to_type: &Text,
    paths: &List<Text>,
) -> Diagnostic {
    // Format paths with detailed information
    let paths_formatted = if !paths.is_empty() {
        paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                let path_type = if path.contains("direct") {
                    "Direct path"
                } else if path.contains("indirect") {
                    "Indirect path"
                } else {
                    "Path"
                };
                format!("  {}. {} - {}", i + 1, path_type, path)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        "  (no paths found)".to_string()
    };

    DiagnosticBuilder::error()
        .code(E0204)
        .message(format!(
            "multiple conversion paths from {} to {}",
            from_type, to_type
        ))
        .span_label(span, format!(
            "ambiguous conversion - {} possible paths",
            paths.len()
        ))
        .add_note(format!(
            "Multiple From implementations create ambiguous conversion paths:\n\n{}",
            paths_formatted
        ))
        .add_note(format!(
            "The '?' operator cannot determine which conversion path to use when \
            converting from {} to {}",
            from_type, to_type
        ))
        .help(format!(
            "Use explicit conversion to disambiguate:\n\
            \n\
            operation()\n\
                .map_err(|e| {}::from(e))?  // Explicitly choose direct path\n\
            \n\
            This makes it clear which From implementation should be used.",
            to_type
        ))
        .help(
            "Remove redundant From implementations:\n\
            \n\
            If you have both direct and indirect conversion paths, consider \
            removing one to eliminate ambiguity. Prefer keeping the most direct path.\n\
            \n\
            Example: Remove either the direct From<ErrorA> for AppError\n\
            or the indirect chain through ErrorB.",
        )
        .help(
            "Use match for explicit error handling:\n\
            \n\
            let value = match operation() {\n\
                Ok(v) => v,\n\
                Err(e) => {\n\
                    // Explicitly choose conversion path\n\
                    return Err(AppError::from(e));\n\
                }\n\
            };",
        )
        .help(
            "Consider using a newtype wrapper to disambiguate:\n\
            \n\
            type SpecificError is ErrorA;\n\
            implement From<SpecificError> for AppError { /* ... */ }\n\
            \n\
            This creates a distinct type that has only one conversion path.",
        )
        .add_note("When multiple From implementations create different conversion paths, the '?' operator cannot choose; use explicit .map_err() to disambiguate or remove redundant From implementations")
        .build()
}

/// Creates a diagnostic for E0205: Cannot use `?` in non-Result context
///
/// This error occurs when the `?` operator is used in a function that doesn't
/// return a Result or Maybe type.
///
/// # Parameters
///
/// - `span`: The location of the `?` operator
/// - `expr_type`: The type of the expression (Result<T, E> or Maybe<T>)
/// - `function_return_type`: The actual return type of the function
/// - `function_name`: Optional name of the function
/// - `function_return_span`: Optional span of the function's return type
///
/// # Example
///
/// ```verum
/// fn compute(x: Int) -> Int {
///     let value = parse_int("42")?;  // E0205: function returns Int, not Result
///     //                           ^
///     value * 2
/// }
/// ```
pub fn e0205_try_in_non_result_context(
    span: Span,
    expr_type: &Text,
    function_return_type: &Text,
    function_name: Option<&Text>,
    function_return_span: Option<Span>,
) -> Diagnostic {
    let func_name_str = function_name
        .map(|n| format!("function '{}'", n))
        .unwrap_or_else(|| "this function".to_string());

    let mut builder = DiagnosticBuilder::error()
        .code(E0205)
        .message(format!(
            "cannot use '?' operator in function returning {}",
            function_return_type
        ))
        .span_label(span, format!("cannot use '?' on {} here", expr_type));

    // Add secondary label for function return type if available
    if let Some(ret_span) = function_return_span {
        builder = builder.secondary_span(
            ret_span,
            format!("{} returns {}", func_name_str, function_return_type),
        );
    }

    builder
        .add_note(
            "The '?' operator can only be used in functions that return Result or Maybe types"
                .to_string(),
        )
        .add_note(format!(
            "{} returns {}, but '?' requires Result<T, E> or Maybe<T>",
            func_name_str, function_return_type
        ))
        .help(format!(
            "Change the function return type to Result:\n\
            \n\
            fn {}(...) -> Result<{}, ErrorType> {{\n\
                // Now you can use '?' operator\n\
                let value = operation()?;\n\
                Ok(value)\n\
            }}",
            function_name.unwrap_or(&Text::from("function")),
            function_return_type
        ))
        .help(
            "Handle the error explicitly without '?':\n\
            \n\
            let value = match operation() {\n\
                Ok(v) => v,\n\
                Err(e) => {\n\
                    // Handle error, return default, or panic\n\
                    return default_value;\n\
                }\n\
            };",
        )
        .help(
            "Use unwrap() if you want to panic on error (not recommended):\n\
            \n\
            let value = operation().unwrap();",
        )
        .help(
            "Use unwrap_or() to provide a default value:\n\
            \n\
            let value = operation().unwrap_or(default_value);",
        )
        .add_note("The '?' operator can only be used in functions returning Result<T, E> or Maybe<T>; change the return type or handle the error explicitly with match/unwrap/unwrap_or")
        .build()
}

/// Creates a diagnostic for nested Result types with `?` operator
///
/// This is a variant of E0205 specifically for nested Result<Result<T, E1>, E2>.
///
/// # Example
///
/// ```verum
/// fn nested_operation() -> Result<Data, AppError> {
///     let result: Result<Result<Data, IoError>, ParseError> = complex_op();
///     let data = result??;  // E0205: nested '?' not allowed
///     Ok(data)
/// }
/// ```
pub fn e0205_nested_try_operator(span: Span, inner_type: &Text, outer_type: &Text) -> Diagnostic {
    DiagnosticBuilder::error()
        .code(E0205)
        .message("nested '?' operators detected")
        .span_label(span, "second '?' operator on nested Result")
        .add_note(format!(
            "The expression has type Result<Result<_, {}>, {}>",
            inner_type, outer_type
        ))
        .add_note("Cannot use '?' operator on nested Result types directly")
        .help(
            "Flatten the nested Result first using and_then:\n\
            \n\
            let data = result.and_then(|r| r)?;",
        )
        .help(
            "Handle both levels explicitly:\n\
            \n\
            let data = match result {\n\
                Ok(inner) => match inner {\n\
                    Ok(data) => data,\n\
                    Err(e) => return Err(/* convert inner error */),\n\
                },\n\
                Err(e) => return Err(/* convert outer error */),\n\
            };",
        )
        .help(
            "Restructure the code to avoid nested Results:\n\
            \n\
            Consider changing complex_op() to return Result<Data, UnifiedError>",
        )
        .add_note("Nested Result<Result<T, E1>, E2> types cannot use chained '??'; flatten with .and_then(|r| r) or restructure to return Result<T, UnifiedError>")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::LineColSpan;

    fn dummy_span() -> Span {
        LineColSpan::new("test.vr", 1, 1, 15)
    }

    #[test]
    fn test_e0203_diagnostic() {
        let diag = e0203_result_type_mismatch(
            dummy_span(),
            &Text::from("IoError"),
            &Text::from("AppError"),
            dummy_span(),
            Some(dummy_span()),
        );

        assert_eq!(diag.code(), Some(E0203));
        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("type mismatch"));
        assert!(diag.message().contains("IoError"));
        assert!(diag.message().contains("AppError"));
        assert!(!diag.helps().is_empty());
        assert!(diag.helps().len() >= 3); // At least 3 fix suggestions
    }

    #[test]
    fn test_e0204_diagnostic() {
        let paths = List::from(vec![
            Text::from("ErrorA -> AppError (direct)"),
            Text::from("ErrorA -> ErrorB -> AppError (indirect)"),
        ]);

        let diag = e0204_multiple_conversion_paths(
            dummy_span(),
            &Text::from("ErrorA"),
            &Text::from("AppError"),
            &paths,
        );

        assert_eq!(diag.code(), Some(E0204));
        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("multiple conversion paths"));
        assert!(!diag.helps().is_empty());
    }

    #[test]
    fn test_e0205_diagnostic() {
        let diag = e0205_try_in_non_result_context(
            dummy_span(),
            &Text::from("Result<Text, IoError>"),
            &Text::from("Int"),
            Some(&Text::from("compute")),
            Some(dummy_span()),
        );

        assert_eq!(diag.code(), Some(E0205));
        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("cannot use '?' operator"));
        assert!(diag.message().contains("Int"));
        assert!(!diag.helps().is_empty());
        assert!(diag.helps().len() >= 3); // At least 3 fix suggestions
    }

    #[test]
    fn test_e0205_nested_diagnostic() {
        let diag = e0205_nested_try_operator(
            dummy_span(),
            &Text::from("IoError"),
            &Text::from("ParseError"),
        );

        assert_eq!(diag.code(), Some(E0205));
        assert_eq!(diag.severity(), Severity::Error);
        assert!(diag.message().contains("nested"));
        assert!(!diag.helps().is_empty());
    }

    #[test]
    fn test_diagnostic_has_multiple_suggestions() {
        let diag = e0203_result_type_mismatch(
            dummy_span(),
            &Text::from("IoError"),
            &Text::from("AppError"),
            dummy_span(),
            None,
        );

        // Verify we have at least 3 actionable suggestions
        assert!(diag.helps().len() >= 3);

        // Verify suggestions contain code examples
        let helps_text: Vec<String> = diag.helps().iter().map(|h| h.message.to_string()).collect();

        // Should have From implementation suggestion
        assert!(helps_text.iter().any(|h| h.contains("implement From")));

        // Should have map_err suggestion
        assert!(helps_text.iter().any(|h| h.contains("map_err")));

        // Should have match suggestion
        assert!(helps_text.iter().any(|h| h.contains("match")));
    }

    #[test]
    fn test_diagnostic_has_inline_guidance() {
        let diag = e0203_result_type_mismatch(
            dummy_span(),
            &Text::from("IoError"),
            &Text::from("AppError"),
            dummy_span(),
            None,
        );

        // Verify notes contain self-contained implementation guidance
        let notes_text: Vec<String> = diag.notes().iter().map(|n| n.message.to_string()).collect();
        assert!(notes_text.iter().any(|n| n.contains("From")));
    }
}
