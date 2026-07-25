//! Specialized error types for refinement type violations.
//!
//! This module provides rich error reporting specifically for refinement type errors,
//! showing actual values that failed constraints and providing actionable suggestions.

use crate::{
    diagnostic::{Diagnostic, DiagnosticBuilder, Span},
    suggestion::{Suggestion, SuggestionBuilder, templates},
};
use serde::{Deserialize, Serialize};
use verum_common::Map;
use verum_common::{List, Text};

/// A constraint expression in a refinement type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    /// The constraint expression (e.g., "i > 0", "x >= 0 && x <= 100")
    pub expression: Text,
    /// The variable being constrained
    pub variable: Text,
}

impl Constraint {
    pub fn new(variable: impl Into<Text>, expression: impl Into<Text>) -> Self {
        Self {
            expression: expression.into(),
            variable: variable.into(),
        }
    }

    /// Parse a constraint from a string like "i > 0"
    pub fn parse(expr: impl Into<Text>) -> Self {
        let expression = expr.into();
        // Simple heuristic: first identifier is the variable
        let variable: Text = expression
            .split_whitespace()
            .first()
            .cloned()
            .unwrap_or_else(|| "value".into());
        Self {
            expression,
            variable,
        }
    }
}

/// A constraint violation with details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintViolation {
    /// The constraint that was violated
    pub constraint: Constraint,
    /// The actual value that violated the constraint
    pub actual_value: Text,
    /// Expected range or condition
    pub expected: Option<Text>,
    /// Additional context about the violation
    pub context: List<Text>,
}

impl ConstraintViolation {
    pub fn new(constraint: Constraint, actual_value: impl Into<Text>) -> Self {
        Self {
            constraint,
            actual_value: actual_value.into(),
            expected: None,
            context: List::new(),
        }
    }

    pub fn with_expected(mut self, expected: impl Into<Text>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn add_context(mut self, context: impl Into<Text>) -> Self {
        self.context.push(context.into());
        self
    }
}

/// A counterexample from SMT solver
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterExample {
    /// Variable assignments that violate the constraint
    pub assignments: Map<Text, Text>,
    /// Trace of how the counterexample was found
    pub trace: Option<Text>,
}

impl CounterExample {
    pub fn new() -> Self {
        Self {
            assignments: Map::new(),
            trace: None,
        }
    }

    pub fn add_assignment(mut self, var: impl Into<Text>, value: impl Into<Text>) -> Self {
        self.assignments.insert(var.into(), value.into());
        self
    }

    pub fn with_trace(mut self, trace: impl Into<Text>) -> Self {
        self.trace = Some(trace.into());
        self
    }
}

impl Default for CounterExample {
    fn default() -> Self {
        Self::new()
    }
}

/// A step in the verification trace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationStep {
    /// Step number (1-indexed)
    pub number: usize,
    /// Type of step: "assumed", "computed", "required", "checking", "proved", "failed"
    pub step_type: Text,
    /// Description of what happened in this step
    pub description: Text,
    /// Related source location
    pub span: Option<Span>,
}

impl VerificationStep {
    pub fn new(number: usize, step_type: impl Into<Text>, description: impl Into<Text>) -> Self {
        Self {
            number,
            step_type: step_type.into(),
            description: description.into(),
            span: None,
        }
    }

    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// SMT verification trace showing the reasoning process
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SMTTrace {
    /// Steps in the verification process
    pub steps: List<VerificationStep>,
    /// Optional counterexample if verification failed
    pub counterexample: Option<CounterExample>,
    /// Whether verification succeeded
    pub succeeded: bool,
}

impl SMTTrace {
    pub fn new(succeeded: bool) -> Self {
        Self {
            steps: List::new(),
            counterexample: None,
            succeeded,
        }
    }

    pub fn add_step(mut self, step: VerificationStep) -> Self {
        self.steps.push(step);
        self
    }

    pub fn with_counterexample(mut self, counterexample: CounterExample) -> Self {
        self.counterexample = Some(counterexample);
        self
    }

    /// Format the trace as a string for display
    pub fn format(&self) -> Text {
        let mut output = Text::new();
        for step in &self.steps {
            output.push_str(&format!(
                "[{}] {}: {}\n",
                step.number, step.step_type, step.description
            ));
        }
        if let Some(ce) = &self.counterexample {
            output.push_str("\nCounterexample found:\n");
            for (var, val) in &ce.assignments {
                output.push_str(&format!("  {} = {}\n", var, val));
            }
        }
        output
    }
}

/// A specialized error for refinement type violations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementError {
    /// The constraint violation
    pub violation: ConstraintViolation,
    /// Source location of the error
    pub span: Span,
    /// SMT verification trace
    pub trace: Option<SMTTrace>,
    /// Suggested fixes
    pub suggestions: List<Suggestion>,
    /// Additional context messages
    pub context: List<Text>,
}

impl RefinementError {
    /// Convert this refinement error to a diagnostic
    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut builder = DiagnosticBuilder::error()
            .code("E0312")
            .message("refinement constraint not satisfied")
            .span_label(
                self.span.clone(),
                format!(
                    "value `{}` fails constraint `{}`",
                    self.violation.actual_value, self.violation.constraint.expression
                ),
            );

        // Add expected value if present
        if let Some(expected) = &self.violation.expected {
            builder = builder.add_note(format!("expected: {}", expected));
        }

        // Add violation context
        for ctx in &self.violation.context {
            builder = builder.add_note(ctx.clone());
        }

        // Add SMT trace if present
        if let Some(trace) = &self.trace
            && !trace.succeeded
        {
            builder = builder.add_note("SMT verification trace:");
            for step in &trace.steps {
                builder = builder.add_note(format!(
                    "  [{}] {}: {}",
                    step.number, step.step_type, step.description
                ));
            }

            if let Some(ce) = &trace.counterexample {
                builder = builder.add_note("Counterexample:");
                for (var, val) in &ce.assignments {
                    builder = builder.add_note(format!("  {} = {}", var, val));
                }
            }
        }

        // Add context messages
        for ctx in &self.context {
            builder = builder.add_note(ctx.clone());
        }

        // Add suggestions as help messages
        for suggestion in &self.suggestions {
            let mut help_msg = suggestion.title().to_string();
            if let Some(snippet) = suggestion.snippet() {
                help_msg.push_str(&format!(":\n  {}", snippet.code));
            }
            builder = builder.help(help_msg);
        }

        builder.build()
    }

    /// Get the constraint that was violated
    pub fn constraint(&self) -> &Constraint {
        &self.violation.constraint
    }

    /// Get the actual value that failed
    pub fn actual_value(&self) -> &str {
        &self.violation.actual_value
    }

    /// Get the span of the error
    pub fn span(&self) -> &Span {
        &self.span
    }
}

/// Builder for refinement errors
pub struct RefinementErrorBuilder {
    constraint: Option<Constraint>,
    actual_value: Option<Text>,
    expected: Option<Text>,
    span: Option<Span>,
    trace: Option<SMTTrace>,
    suggestions: List<Suggestion>,
    context: List<Text>,
    violation_context: List<Text>,
}

impl RefinementErrorBuilder {
    pub fn new() -> Self {
        Self {
            constraint: None,
            actual_value: None,
            expected: None,
            span: None,
            trace: None,
            suggestions: List::new(),
            context: List::new(),
            violation_context: List::new(),
        }
    }

    pub fn constraint(mut self, constraint: impl Into<Text>) -> Self {
        self.constraint = Some(Constraint::parse(constraint));
        self
    }

    pub fn constraint_obj(mut self, constraint: Constraint) -> Self {
        self.constraint = Some(constraint);
        self
    }

    pub fn actual_value(mut self, value: impl Into<Text>) -> Self {
        self.actual_value = Some(value.into());
        self
    }

    pub fn expected(mut self, expected: impl Into<Text>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    pub fn span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    pub fn trace(mut self, trace: SMTTrace) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn suggestion(mut self, suggestion: impl Into<Text>) -> Self {
        self.suggestions.push(
            SuggestionBuilder::new(suggestion.into())
                .alternative()
                .build(),
        );
        self
    }

    pub fn suggestion_obj(mut self, suggestion: Suggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    pub fn context(mut self, context: impl Into<Text>) -> Self {
        self.context.push(context.into());
        self
    }

    pub fn violation_context(mut self, context: impl Into<Text>) -> Self {
        self.violation_context.push(context.into());
        self
    }

    pub fn build(self) -> RefinementError {
        let constraint = self.constraint.unwrap_or_else(|| Constraint::parse("true"));
        let actual_value = self.actual_value.unwrap_or_else(|| "unknown".into());
        let span = self
            .span
            .unwrap_or_else(|| Span::new("<no-location>", 0, 0, 0));

        let mut violation = ConstraintViolation::new(constraint, actual_value);
        if let Some(expected) = self.expected {
            violation = violation.with_expected(expected);
        }
        for ctx in self.violation_context {
            violation = violation.add_context(ctx);
        }

        RefinementError {
            violation,
            span,
            trace: self.trace,
            suggestions: self.suggestions,
            context: self.context,
        }
    }
}

impl Default for RefinementErrorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper functions for creating common refinement errors
pub mod common {
    use super::*;

    /// Create an error for a value that should be positive
    pub fn positive_constraint_violation(
        var_name: &str,
        actual_value: &str,
        span: Span,
    ) -> RefinementError {
        RefinementErrorBuilder::new()
            .constraint_obj(Constraint::new(var_name, format!("{} > 0", var_name)))
            .actual_value(actual_value)
            .expected("positive value")
            .span(span.clone())
            .suggestion_obj(templates::add_refinement_constraint(var_name, "> 0"))
            .suggestion_obj(templates::runtime_check(
                &format!("{} > 0", var_name),
                "return Err(\"value must be positive\")",
            ))
            .suggestion_obj(templates::compile_time_proof(&format!("{} > 0", var_name)))
            .build()
    }

    /// Create an error for array bounds violation
    pub fn bounds_check_violation(
        array: &str,
        index: &str,
        actual_index: &str,
        array_len: &str,
        span: Span,
    ) -> RefinementError {
        RefinementErrorBuilder::new()
            .constraint_obj(Constraint::new(
                index,
                format!("{} < {}.len()", index, array),
            ))
            .actual_value(actual_index)
            .expected(format!("0 <= {} < {}", index, array_len))
            .span(span.clone())
            .suggestion_obj(templates::add_refinement_constraint(
                index,
                &format!("< {}.len()", array),
            ))
            .suggestion_obj(templates::runtime_check(
                &format!("{} < {}.len()", index, array),
                "return None",
            ))
            .suggestion_obj(templates::use_safe_method(
                "get",
                "Use Vec::get() which returns Option<T>",
            ))
            .build()
    }

    /// Create an error for division by zero
    pub fn division_by_zero(divisor: &str, actual_value: &str, span: Span) -> RefinementError {
        RefinementErrorBuilder::new()
            .constraint_obj(Constraint::new(divisor, format!("{} != 0", divisor)))
            .actual_value(actual_value)
            .expected("non-zero value")
            .span(span.clone())
            .suggestion_obj(templates::add_refinement_constraint(divisor, "!= 0"))
            .suggestion_obj(templates::runtime_check(
                &format!("{} != 0", divisor),
                "return Err(DivisionByZero)",
            ))
            .suggestion_obj(templates::use_safe_method(
                "checked_div",
                "Use checked_div() which returns Option<T>",
            ))
            .build()
    }

    /// Create an error for range constraint violation
    pub fn range_violation(
        var_name: &str,
        actual_value: &str,
        min: &str,
        max: &str,
        span: Span,
    ) -> RefinementError {
        RefinementErrorBuilder::new()
            .constraint_obj(Constraint::new(
                var_name,
                format!("{} >= {} && {} <= {}", var_name, min, var_name, max),
            ))
            .actual_value(actual_value)
            .expected(format!("{} <= value <= {}", min, max))
            .span(span.clone())
            .suggestion_obj(templates::add_refinement_constraint(
                var_name,
                &format!(">= {} && <= {}", min, max),
            ))
            .suggestion_obj(templates::runtime_check(
                &format!("{} >= {} && {} <= {}", var_name, min, var_name, max),
                "return Err(OutOfRange)",
            ))
            .build()
    }
}
