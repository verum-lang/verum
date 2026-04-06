//! Hygiene Violation Types
//!
//! Defines all types of hygiene violations that can occur during macro expansion.
//!
//! Violations: AccidentalCapture (M402), ScopeResolutionFailure (M404),
//! StageMismatch (M405), UnresolvedSplice, DuplicateBinding, ShadowingWarning.
//! Each violation tracks the identifier, its mark set, and source span.

use verum_ast::Span;
use verum_common::Text;

use super::scope::HygienicIdent;

/// Types of hygiene violations that can occur during macro expansion
#[derive(Debug, Clone)]
pub enum HygieneViolation {
    /// Attempted to capture a variable from expansion site
    AccidentalCapture {
        /// The captured identifier
        captured: HygienicIdent,
        /// Where the binding was intended to come from
        intended_binding: Span,
        /// Where the binding was actually found
        actual_binding: Span,
    },

    /// Variable shadowing in generated code conflicts with caller
    ShadowConflict {
        /// The shadowed identifier
        shadowed: HygienicIdent,
        /// Where the shadowing occurred
        introduced_at: Span,
    },

    /// Unquote ($) used outside quote context
    UnquoteOutsideQuote {
        /// Location of the unquote
        span: Span,
    },

    /// Stage mismatch in quote/unquote
    StageMismatch {
        /// Expected stage level
        expected_stage: u32,
        /// Actual stage level
        actual_stage: u32,
        /// Location of the mismatch
        span: Span,
    },

    /// Cannot lift value of this type
    LiftTypeMismatch {
        /// Expected liftable type
        expected: Text,
        /// Actual type found
        found: Text,
        /// Location
        span: Span,
    },

    /// Invalid quote syntax
    InvalidQuoteSyntax {
        /// Description of the syntax error
        message: Text,
        /// Location
        span: Span,
    },

    /// Scope resolution failed
    ScopeResolutionFailed {
        /// The identifier that couldn't be resolved
        ident: Text,
        /// Location
        span: Span,
    },

    /// Capture not declared (for strict mode)
    CaptureNotDeclared {
        /// The undeclared capture
        ident: Text,
        /// Location
        span: Span,
    },

    /// Repetition length mismatch
    RepetitionMismatch {
        /// First list name and length
        first_name: Text,
        first_len: usize,
        /// Second list name and length
        second_name: Text,
        second_len: usize,
        /// Location
        span: Span,
    },

    /// Gensym collision (internal error)
    GensymCollision {
        /// The colliding name
        name: Text,
        /// Location
        span: Span,
    },

    /// Invalid token tree
    InvalidTokenTree {
        /// Description of the error
        message: Text,
        /// Location
        span: Span,
    },
}

impl HygieneViolation {
    /// Get the error code for this violation
    ///
    /// Error codes follow the M4xx range for meta-system errors.
    pub fn error_code(&self) -> &'static str {
        match self {
            HygieneViolation::InvalidQuoteSyntax { .. } => "M400",
            HygieneViolation::UnquoteOutsideQuote { .. } => "M401",
            HygieneViolation::AccidentalCapture { .. } => "M402",
            HygieneViolation::ShadowConflict { .. } => "M402",
            HygieneViolation::GensymCollision { .. } => "M403",
            HygieneViolation::ScopeResolutionFailed { .. } => "M404",
            HygieneViolation::StageMismatch { .. } => "M405",
            HygieneViolation::LiftTypeMismatch { .. } => "M406",
            HygieneViolation::InvalidTokenTree { .. } => "M407",
            HygieneViolation::CaptureNotDeclared { .. } => "M408",
            HygieneViolation::RepetitionMismatch { .. } => "M409",
        }
    }

    /// Get the span where this violation occurred
    pub fn span(&self) -> Span {
        match self {
            HygieneViolation::AccidentalCapture { captured, .. } => captured.span,
            HygieneViolation::ShadowConflict { introduced_at, .. } => *introduced_at,
            HygieneViolation::UnquoteOutsideQuote { span } => *span,
            HygieneViolation::StageMismatch { span, .. } => *span,
            HygieneViolation::LiftTypeMismatch { span, .. } => *span,
            HygieneViolation::InvalidQuoteSyntax { span, .. } => *span,
            HygieneViolation::ScopeResolutionFailed { span, .. } => *span,
            HygieneViolation::CaptureNotDeclared { span, .. } => *span,
            HygieneViolation::RepetitionMismatch { span, .. } => *span,
            HygieneViolation::GensymCollision { span, .. } => *span,
            HygieneViolation::InvalidTokenTree { span, .. } => *span,
        }
    }

    /// Get a human-readable message for this violation
    pub fn message(&self) -> Text {
        match self {
            HygieneViolation::AccidentalCapture { captured, .. } => {
                Text::from(format!(
                    "Accidental variable capture: '{}' was captured from the expansion site",
                    captured.name.as_str()
                ))
            }
            HygieneViolation::ShadowConflict { shadowed, .. } => {
                Text::from(format!(
                    "Variable shadowing conflict: '{}' shadows a binding from the call site",
                    shadowed.name.as_str()
                ))
            }
            HygieneViolation::UnquoteOutsideQuote { .. } => {
                Text::from("Unquote ($) used outside of a quote expression")
            }
            HygieneViolation::StageMismatch {
                expected_stage,
                actual_stage,
                ..
            } => {
                Text::from(format!(
                    "Stage mismatch: expected stage {}, found stage {}",
                    expected_stage, actual_stage
                ))
            }
            HygieneViolation::LiftTypeMismatch { expected, found, .. } => {
                Text::from(format!(
                    "Cannot lift value: expected liftable type {}, found {}",
                    expected.as_str(),
                    found.as_str()
                ))
            }
            HygieneViolation::InvalidQuoteSyntax { message, .. } => message.clone(),
            HygieneViolation::ScopeResolutionFailed { ident, .. } => {
                Text::from(format!(
                    "Cannot resolve identifier '{}' in this scope",
                    ident.as_str()
                ))
            }
            HygieneViolation::CaptureNotDeclared { ident, .. } => {
                Text::from(format!(
                    "Variable '{}' is referenced but not in scope or captured",
                    ident.as_str()
                ))
            }
            HygieneViolation::RepetitionMismatch {
                first_name,
                first_len,
                second_name,
                second_len,
                ..
            } => {
                Text::from(format!(
                    "Repetition length mismatch: '{}' has {} elements, '{}' has {} elements",
                    first_name.as_str(),
                    first_len,
                    second_name.as_str(),
                    second_len
                ))
            }
            HygieneViolation::GensymCollision { name, .. } => {
                Text::from(format!(
                    "Internal error: generated name '{}' collides with existing identifier",
                    name.as_str()
                ))
            }
            HygieneViolation::InvalidTokenTree { message, .. } => {
                Text::from(format!("Invalid token tree: {}", message.as_str()))
            }
        }
    }

    /// Get a detailed diagnostic message with context
    pub fn detailed_message(&self) -> Text {
        let code = self.error_code();
        let msg = self.message();
        Text::from(format!("[{}] {}", code, msg.as_str()))
    }

    /// Check if this violation is fatal (prevents successful compilation)
    pub fn is_fatal(&self) -> bool {
        match self {
            // All hygiene violations are fatal by default
            _ => true,
        }
    }

    /// Check if this violation can be recovered from
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Some violations might be recoverable in lenient mode
            HygieneViolation::ShadowConflict { .. } => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for HygieneViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detailed_message().as_str())
    }
}

impl std::error::Error for HygieneViolation {}

/// Collection of hygiene violations
#[derive(Debug, Clone, Default)]
pub struct HygieneViolations {
    violations: Vec<HygieneViolation>,
}

impl HygieneViolations {
    /// Create an empty collection
    pub fn new() -> Self {
        Self {
            violations: Vec::new(),
        }
    }

    /// Add a violation
    pub fn push(&mut self, violation: HygieneViolation) {
        self.violations.push(violation);
    }

    /// Check if there are any violations
    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    /// Get the number of violations
    pub fn len(&self) -> usize {
        self.violations.len()
    }

    /// Iterate over violations
    pub fn iter(&self) -> impl Iterator<Item = &HygieneViolation> {
        self.violations.iter()
    }

    /// Convert to a Vec
    pub fn into_vec(self) -> Vec<HygieneViolation> {
        self.violations
    }

    /// Get all fatal violations
    pub fn fatal_violations(&self) -> impl Iterator<Item = &HygieneViolation> {
        self.violations.iter().filter(|v| v.is_fatal())
    }

    /// Check if there are any fatal violations
    pub fn has_fatal(&self) -> bool {
        self.violations.iter().any(|v| v.is_fatal())
    }

    /// Convert to a Result
    pub fn into_result(self) -> Result<(), Self> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }
}

impl IntoIterator for HygieneViolations {
    type Item = HygieneViolation;
    type IntoIter = std::vec::IntoIter<HygieneViolation>;

    fn into_iter(self) -> Self::IntoIter {
        self.violations.into_iter()
    }
}

impl FromIterator<HygieneViolation> for HygieneViolations {
    fn from_iter<T: IntoIterator<Item = HygieneViolation>>(iter: T) -> Self {
        Self {
            violations: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes() {
        let unquote = HygieneViolation::UnquoteOutsideQuote {
            span: Span::default(),
        };
        assert_eq!(unquote.error_code(), "M401");

        let stage = HygieneViolation::StageMismatch {
            expected_stage: 1,
            actual_stage: 0,
            span: Span::default(),
        };
        assert_eq!(stage.error_code(), "M405");
    }

    #[test]
    fn test_violations_collection() {
        let mut violations = HygieneViolations::new();
        assert!(violations.is_empty());

        violations.push(HygieneViolation::UnquoteOutsideQuote {
            span: Span::default(),
        });
        assert_eq!(violations.len(), 1);
        assert!(violations.has_fatal());
    }
}
