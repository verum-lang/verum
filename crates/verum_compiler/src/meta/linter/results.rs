//! Lint Results for Meta Linter
//!
//! Contains LintResult, LintWarning, LintError, and UnsafePattern types.
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use verum_ast::Span;
use verum_common::{List, Maybe, Text};

use super::patterns::UnsafePatternKind;

/// Result of linting a meta function
#[derive(Debug, Clone)]
pub struct LintResult {
    /// Whether the function is considered safe
    pub is_safe: bool,
    /// Detected unsafe patterns
    pub unsafe_patterns: List<UnsafePattern>,
    /// Warnings generated during linting
    pub warnings: List<LintWarning>,
    /// Errors that require annotation or fix
    pub errors: List<LintError>,
}

impl LintResult {
    /// Create an empty (safe) lint result
    pub fn safe() -> Self {
        Self {
            is_safe: true,
            unsafe_patterns: List::new(),
            warnings: List::new(),
            errors: List::new(),
        }
    }

    /// Create an unsafe lint result with patterns
    pub fn unsafe_with_patterns(patterns: List<UnsafePattern>) -> Self {
        Self {
            is_safe: false,
            unsafe_patterns: patterns,
            warnings: List::new(),
            errors: List::new(),
        }
    }

    /// Add a warning
    pub fn add_warning(&mut self, warning: LintWarning) {
        self.warnings.push(warning);
    }

    /// Add an error
    pub fn add_error(&mut self, error: LintError) {
        self.errors.push(error);
        self.is_safe = false;
    }

    /// Check if there are any issues
    pub fn has_issues(&self) -> bool {
        !self.unsafe_patterns.is_empty() || !self.warnings.is_empty() || !self.errors.is_empty()
    }
}

/// An unsafe pattern detected in meta code
#[derive(Debug, Clone)]
pub struct UnsafePattern {
    /// Kind of unsafe pattern
    pub kind: UnsafePatternKind,
    /// Description of the issue
    pub description: Text,
    /// Source location
    pub span: Span,
    /// Suggested fix
    pub suggestion: Maybe<Text>,
}

/// A lint warning (non-blocking)
#[derive(Debug, Clone)]
pub struct LintWarning {
    pub message: Text,
    pub span: Span,
    pub suggestion: Maybe<Text>,
}

/// A lint error (blocking)
#[derive(Debug, Clone)]
pub struct LintError {
    pub message: Text,
    pub span: Span,
    pub help: Maybe<Text>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_lint_result() {
        let result = LintResult::safe();
        assert!(result.is_safe);
        assert!(result.unsafe_patterns.is_empty());
        assert!(result.warnings.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_lint_result_add_warning() {
        let mut result = LintResult::safe();
        result.add_warning(LintWarning {
            message: Text::from("test warning"),
            span: Span::default(),
            suggestion: Maybe::None,
        });
        assert!(result.has_issues());
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn test_lint_result_add_error() {
        let mut result = LintResult::safe();
        result.add_error(LintError {
            message: Text::from("test error"),
            span: Span::default(),
            help: Maybe::None,
        });
        assert!(!result.is_safe);
        assert!(result.has_issues());
        assert_eq!(result.errors.len(), 1);
    }
}
