//! Exhaustiveness Diagnostics
//!
//! This module provides error and warning types for exhaustiveness checking,
//! with helpful messages for developers.

use super::witness::Witness;
use std::fmt;
use verum_common::{List, Text};

/// Error code for exhaustiveness-related errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExhaustivenessErrorCode {
    /// E0601: Non-exhaustive pattern match
    NonExhaustive,
    /// E0604: Invalid pattern for type
    InvalidPattern,
}

impl fmt::Display for ExhaustivenessErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExhaustivenessErrorCode::NonExhaustive => write!(f, "E0601"),
            ExhaustivenessErrorCode::InvalidPattern => write!(f, "E0604"),
        }
    }
}

/// Warning code for exhaustiveness-related warnings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExhaustivenessWarningCode {
    /// W0602: Unreachable pattern
    Unreachable,
    /// W0603: Match with all guarded patterns
    AllGuarded,
    /// W0605: TypeTest pattern on concrete type
    RedundantTypeTest,
    /// W0606: Range pattern overlaps with another pattern
    RangeOverlap,
    /// W0607: Range pattern is completely redundant (subset of another)
    RedundantRange,
}

impl fmt::Display for ExhaustivenessWarningCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExhaustivenessWarningCode::Unreachable => write!(f, "W0602"),
            ExhaustivenessWarningCode::AllGuarded => write!(f, "W0603"),
            ExhaustivenessWarningCode::RedundantTypeTest => write!(f, "W0605"),
            ExhaustivenessWarningCode::RangeOverlap => write!(f, "W0606"),
            ExhaustivenessWarningCode::RedundantRange => write!(f, "W0607"),
        }
    }
}

/// An error from exhaustiveness checking
#[derive(Debug, Clone)]
pub struct ExhaustivenessError {
    pub code: ExhaustivenessErrorCode,
    pub message: Text,
    pub witnesses: List<Witness>,
    pub span: Option<verum_ast::span::Span>,
    pub suggestions: List<Text>,
}

impl ExhaustivenessError {
    /// Create a non-exhaustive error
    pub fn non_exhaustive(witnesses: List<Witness>, span: Option<verum_ast::span::Span>) -> Self {
        let witness_str = witnesses
            .iter()
            .take(3)
            .map(|w| format!("`{}`", w))
            .collect::<Vec<_>>()
            .join(", ");

        let message = if witnesses.len() <= 3 {
            Text::from(format!(
                "non-exhaustive patterns: {} not covered",
                witness_str
            ))
        } else {
            Text::from(format!(
                "non-exhaustive patterns: {} and {} other(s) not covered",
                witness_str,
                witnesses.len() - 3
            ))
        };

        let suggestions = vec![
            Text::from("ensure that all possible cases are covered"),
            Text::from("consider adding a wildcard pattern `_` as the last arm"),
        ];

        Self {
            code: ExhaustivenessErrorCode::NonExhaustive,
            message,
            witnesses,
            span,
            suggestions: List::from_iter(suggestions),
        }
    }

    /// Create an invalid pattern error
    pub fn invalid_pattern(
        reason: impl Into<Text>,
        span: Option<verum_ast::span::Span>,
    ) -> Self {
        Self {
            code: ExhaustivenessErrorCode::InvalidPattern,
            message: reason.into(),
            witnesses: List::new(),
            span,
            suggestions: List::new(),
        }
    }
}

impl fmt::Display for ExhaustivenessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error[{}]: {}", self.code, self.message)?;
        for suggestion in self.suggestions.iter() {
            write!(f, "\n  = help: {}", suggestion)?;
        }
        Ok(())
    }
}

/// A warning from exhaustiveness checking
#[derive(Debug, Clone)]
pub struct ExhaustivenessWarning {
    pub code: ExhaustivenessWarningCode,
    pub message: Text,
    pub span: Option<verum_ast::span::Span>,
    pub pattern_index: Option<usize>,
}

impl ExhaustivenessWarning {
    /// Create an unreachable pattern warning
    pub fn unreachable(pattern_index: usize, span: Option<verum_ast::span::Span>) -> Self {
        Self {
            code: ExhaustivenessWarningCode::Unreachable,
            message: Text::from("unreachable pattern"),
            span,
            pattern_index: Some(pattern_index),
        }
    }

    /// Create an all-guarded warning
    pub fn all_guarded(span: Option<verum_ast::span::Span>) -> Self {
        Self {
            code: ExhaustivenessWarningCode::AllGuarded,
            message: Text::from(
                "match expression may not be exhaustive: all arms have guards. \
                 If all guards evaluate to false, no arm will match. \
                 Consider adding an `else` branch or a pattern without a guard.",
            ),
            span,
            pattern_index: None,
        }
    }

    /// Create a redundant TypeTest warning
    pub fn redundant_type_test(
        tested_type: impl Into<Text>,
        actual_type: impl Into<Text>,
        span: Option<verum_ast::span::Span>,
    ) -> Self {
        Self {
            code: ExhaustivenessWarningCode::RedundantTypeTest,
            message: Text::from(format!(
                "TypeTest pattern for `{}` is redundant: scrutinee has type `{}`",
                tested_type.into(),
                actual_type.into()
            )),
            span,
            pattern_index: None,
        }
    }

    /// Create a range overlap warning
    pub fn range_overlap(
        first_index: usize,
        second_index: usize,
        overlap_start: i128,
        overlap_end: i128,
        span: Option<verum_ast::span::Span>,
    ) -> Self {
        let overlap_desc = if overlap_start == overlap_end {
            format!("value {}", overlap_start)
        } else {
            format!("range {}..={}", overlap_start, overlap_end)
        };

        Self {
            code: ExhaustivenessWarningCode::RangeOverlap,
            message: Text::from(format!(
                "range patterns {} and {} overlap on {}",
                first_index + 1,
                second_index + 1,
                overlap_desc
            )),
            span,
            pattern_index: Some(second_index),
        }
    }

    /// Create a redundant range warning
    pub fn redundant_range(
        redundant_index: usize,
        covering_index: usize,
        span: Option<verum_ast::span::Span>,
    ) -> Self {
        Self {
            code: ExhaustivenessWarningCode::RedundantRange,
            message: Text::from(format!(
                "range pattern {} is completely covered by pattern {} and is unreachable",
                redundant_index + 1,
                covering_index + 1
            )),
            span,
            pattern_index: Some(redundant_index),
        }
    }
}

impl fmt::Display for ExhaustivenessWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "warning[{}]: {}", self.code, self.message)
    }
}

/// Format witnesses for display in error messages
pub fn format_witnesses(witnesses: &[Witness], max_count: usize) -> String {
    if witnesses.is_empty() {
        return String::from("(no examples)");
    }

    let shown: Vec<_> = witnesses.iter().take(max_count).collect();
    let more = witnesses.len().saturating_sub(max_count);

    let formatted = shown
        .iter()
        .map(|w| format!("`{}`", w))
        .collect::<Vec<_>>()
        .join(", ");

    if more > 0 {
        format!("{} and {} more", formatted, more)
    } else {
        formatted
    }
}

/// Generate a suggestion for adding missing patterns
pub fn suggest_missing_patterns(witnesses: &[Witness]) -> String {
    let arms: Vec<_> = witnesses
        .iter()
        .take(5)
        .map(|w| format!("    {} => todo!(),", w))
        .collect();

    if witnesses.len() > 5 {
        format!(
            "consider adding arms for:\n{}\n    // ... and {} more",
            arms.join("\n"),
            witnesses.len() - 5
        )
    } else {
        format!("consider adding arms for:\n{}", arms.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let error = ExhaustivenessError::non_exhaustive(
            List::from_iter([Witness::Constructor {
                name: Text::from("None"),
                args: List::new(),
            }]),
            None,
        );
        let display = format!("{}", error);
        assert!(display.contains("E0601"));
        assert!(display.contains("None"));
    }

    #[test]
    fn test_warning_display() {
        let warning = ExhaustivenessWarning::unreachable(2, None);
        let display = format!("{}", warning);
        assert!(display.contains("W0602"));
        assert!(display.contains("unreachable"));
    }

    #[test]
    fn test_format_witnesses() {
        let witnesses = vec![
            Witness::Constructor {
                name: Text::from("A"),
                args: List::new(),
            },
            Witness::Constructor {
                name: Text::from("B"),
                args: List::new(),
            },
        ];
        let formatted = format_witnesses(&witnesses, 3);
        assert!(formatted.contains("A"));
        assert!(formatted.contains("B"));
    }
}
