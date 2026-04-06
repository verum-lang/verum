//! Attribute validation errors.
//!
//! This module defines error types for attribute validation,
//! with rich diagnostic information for IDE and compiler integration.

use verum_ast::attr::{ArgValidationError, AttributeTarget};
use verum_ast::span::Span;
use verum_common::span::LineColSpan;
use verum_common::{List, Text};
use verum_diagnostics::Diagnostic;

/// Convert AST Span to diagnostic LineColSpan.
///
/// Uses file_id and byte offsets since we don't have full source info here.
fn span_to_linecol(span: Span) -> LineColSpan {
    LineColSpan::new(
        format!("file_{}", span.file_id.raw()),
        span.start as usize, // Use byte offset as line placeholder
        0,                   // Column
        span.end as usize,   // End offset
    )
}

/// Error during attribute validation.
#[derive(Debug, Clone)]
pub enum AttributeError {
    /// Unknown attribute name
    Unknown {
        attr: Text,
        span: Span,
        suggestions: List<Text>,
    },

    /// Attribute not valid for this target
    InvalidTarget {
        attr: Text,
        target: AttributeTarget,
        valid_targets: AttributeTarget,
        span: Span,
    },

    /// Invalid arguments
    InvalidArgs {
        attr: Text,
        error: ArgValidationError,
        span: Span,
    },

    /// Duplicate non-repeatable attribute
    Duplicate {
        attr: Text,
        first_span: Span,
        second_span: Span,
    },

    /// Conflicting attributes
    Conflict {
        attr1: Text,
        attr2: Text,
        span: Span,
    },

    /// Missing required attribute
    MissingRequirement {
        attr: Text,
        requires: Text,
        span: Span,
    },

    /// Feature gate not enabled
    FeatureGateRequired {
        attr: Text,
        feature: Text,
        span: Span,
    },
}

impl AttributeError {
    /// Get a human-readable error message.
    #[must_use]
    pub fn message(&self) -> Text {
        match self {
            Self::Unknown {
                attr, suggestions, ..
            } => {
                if suggestions.is_empty() {
                    Text::from(format!("unknown attribute `@{}`", attr))
                } else {
                    Text::from(format!(
                        "unknown attribute `@{}`; did you mean {}?",
                        attr,
                        suggestions
                            .iter()
                            .map(|s| format!("`@{}`", s))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            Self::InvalidTarget {
                attr,
                target,
                valid_targets,
                ..
            } => Text::from(format!(
                "`@{}` is not valid on {}; valid targets: {}",
                attr,
                target.display_name(),
                valid_targets.format_list()
            )),
            Self::InvalidArgs { attr, error, .. } => {
                Text::from(format!("`@{}`: {}", attr, error.message()))
            }
            Self::Duplicate { attr, .. } => Text::from(format!("`@{}` can only appear once", attr)),
            Self::Conflict { attr1, attr2, .. } => {
                Text::from(format!("`@{}` conflicts with `@{}`", attr1, attr2))
            }
            Self::MissingRequirement { attr, requires, .. } => {
                Text::from(format!("`@{}` requires `@{}`", attr, requires))
            }
            Self::FeatureGateRequired { attr, feature, .. } => Text::from(format!(
                "`@{}` requires feature `{}` to be enabled",
                attr, feature
            )),
        }
    }

    /// Get the primary span for this error.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::Unknown { span, .. } => *span,
            Self::InvalidTarget { span, .. } => *span,
            Self::InvalidArgs { span, .. } => *span,
            Self::Duplicate { second_span, .. } => *second_span,
            Self::Conflict { span, .. } => *span,
            Self::MissingRequirement { span, .. } => *span,
            Self::FeatureGateRequired { span, .. } => *span,
        }
    }

    /// Convert to a diagnostic for display.
    #[must_use]
    pub fn to_diagnostic(&self) -> Diagnostic {
        // Create the base diagnostic with converted span
        

        // Note: Additional information like secondary spans and suggestions
        // would require using DiagnosticBuilder, but for now we return the base diagnostic.
        // The message() method already includes the most important information.
        Diagnostic::new_error(self.message(), span_to_linecol(self.span()), self.code())
    }

    /// Get the primary label for the error span.
    fn primary_label(&self) -> Text {
        match self {
            Self::Unknown { .. } => Text::from("unknown attribute"),
            Self::InvalidTarget { target, .. } => {
                Text::from(format!("not valid on {}", target.display_name()))
            }
            Self::InvalidArgs { .. } => Text::from("invalid arguments"),
            Self::Duplicate { .. } => Text::from("duplicate attribute"),
            Self::Conflict { attr2, .. } => Text::from(format!("conflicts with `@{}`", attr2)),
            Self::MissingRequirement { requires, .. } => {
                Text::from(format!("requires `@{}`", requires))
            }
            Self::FeatureGateRequired { feature, .. } => {
                Text::from(format!("requires feature `{}`", feature))
            }
        }
    }

    /// Get the error code for this error.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => "E0400",
            Self::InvalidTarget { .. } => "E0401",
            Self::InvalidArgs { .. } => "E0402",
            Self::Duplicate { .. } => "E0403",
            Self::Conflict { .. } => "E0404",
            Self::MissingRequirement { .. } => "E0405",
            Self::FeatureGateRequired { .. } => "E0406",
        }
    }
}

impl std::fmt::Display for AttributeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for AttributeError {}

/// Convert a list of attribute errors to diagnostics.
pub fn errors_to_diagnostics(errors: &[AttributeError]) -> List<Diagnostic> {
    errors.iter().map(|e| e.to_diagnostic()).collect()
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_error_message() {
        let err = AttributeError::Unknown {
            attr: Text::from("unknwon"),
            span: Span::default(),
            suggestions: vec![Text::from("unknown")].into(),
        };
        let msg = err.message();
        assert!(msg.as_str().contains("unknwon"));
        assert!(msg.as_str().contains("did you mean"));
    }

    #[test]
    fn test_invalid_target_message() {
        let err = AttributeError::InvalidTarget {
            attr: Text::from("inline"),
            target: AttributeTarget::Field,
            valid_targets: AttributeTarget::Function,
            span: Span::default(),
        };
        let msg = err.message();
        assert!(msg.as_str().contains("inline"));
        assert!(msg.as_str().contains("field"));
        assert!(msg.as_str().contains("function"));
    }

    #[test]
    fn test_error_codes() {
        let err = AttributeError::Unknown {
            attr: Text::from("x"),
            span: Span::default(),
            suggestions: vec![].into(),
        };
        assert_eq!(err.code(), "E0400");
    }

    #[test]
    fn test_to_diagnostic() {
        let err = AttributeError::Conflict {
            attr1: Text::from("cold"),
            attr2: Text::from("hot"),
            span: Span::default(),
        };
        let diag = err.to_diagnostic();
        // Diagnostic was created successfully
        assert!(diag.message().contains("cold"));
    }
}
