//! Enhanced Refinement Error Diagnostics
//!
//! Refinement type error diagnostics: generates detailed error messages for failed refinement
//! checks including source locations, predicate details, and suggested fixes. Handles both
//! compile-time (proof mode) and runtime validation failure reporting.
//!
//! This module implements production-grade error diagnostics for refinement type violations
//! with the following features:
//! - Actual value tracking and display
//! - Predicate evaluation and decomposition
//! - Context-aware suggestion generation
//! - Multi-constraint breakdown with ✓/✗ markers
//! - Nested refinement error reporting
//!
//! # Quality Standards (Spec §8.2)
//!
//! All refinement errors MUST provide:
//! 1. Actual value that failed
//! 2. Specific constraint violated
//! 3. Actionable suggestions
//! 4. Context (where constraint came from)
//! 5. Nested refinement support

use std::fmt::{self, Display, Formatter};
use verum_ast::{
    expr::{BinOp, Expr, ExprKind, UnOp},
    span::Span,
};
use verum_common::{ConstValue, List, Maybe, Text};
use verum_common::ToText;

// ==================== Core Types ====================

/// Source of a refinement constraint (Spec §8.2 Lines 13168-13173)
#[derive(Debug, Clone, PartialEq)]
pub enum RefinementSource {
    /// From explicit type annotation: `let x: Positive = value`
    TypeAnnotation,
    /// From function parameter: `fn f(x: Positive)`
    FunctionParameter,
    /// From function return type: `fn f() -> Positive`
    FunctionReturn,
    /// From record field: `{ x: Positive }`
    FieldConstraint,
    /// From variable assignment
    Assignment,
}

impl Display for RefinementSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RefinementSource::TypeAnnotation => write!(f, "type annotation"),
            RefinementSource::FunctionParameter => write!(f, "function parameter"),
            RefinementSource::FunctionReturn => write!(f, "function return type"),
            RefinementSource::FieldConstraint => write!(f, "field constraint"),
            RefinementSource::Assignment => write!(f, "assignment"),
        }
    }
}

/// Context for error messages (Spec §8.2 Lines 13160-13166)
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Function name if error is in function context
    pub function_name: Maybe<Text>,
    /// Expected type with refinement
    pub expected_type: Text,
    /// Actual type without refinement
    pub actual_type: Text,
    /// Source of the refinement
    pub refinement_source: RefinementSource,
}

/// Result of evaluating a single constraint (Spec §8.3)
#[derive(Debug, Clone, PartialEq)]
pub enum ConstraintResult {
    /// Constraint satisfied
    Satisfied,
    /// Constraint violated
    Violated,
    /// Cannot determine (insufficient information)
    Unknown,
}

impl ConstraintResult {
    /// Get display marker (✓ or ✗)
    pub fn marker(&self) -> &'static str {
        match self {
            ConstraintResult::Satisfied => "✓",
            ConstraintResult::Violated => "✗",
            ConstraintResult::Unknown => "?",
        }
    }

    pub fn is_violated(&self) -> bool {
        matches!(self, ConstraintResult::Violated)
    }
}

/// Evaluation of a single constraint within a compound predicate
#[derive(Debug, Clone)]
pub struct ConstraintEvaluation {
    /// The constraint expression as string
    pub expression: Text,
    /// Evaluation result
    pub result: ConstraintResult,
    /// Explanation of the result
    pub explanation: Maybe<Text>,
}

impl ConstraintEvaluation {
    pub fn new(expression: Text, result: ConstraintResult) -> Self {
        Self {
            expression,
            result,
            explanation: Maybe::None,
        }
    }

    pub fn with_explanation(mut self, explanation: Text) -> Self {
        self.explanation = Maybe::Some(explanation);
        self
    }

    /// Format as a single line with marker
    pub fn format_line(&self) -> Text {
        let marker = self.result.marker();
        match &self.explanation {
            Maybe::Some(exp) => Text::from(format!("{} {}: {}", marker, self.expression, exp)),
            Maybe::None => Text::from(format!("{} {}", marker, self.expression)),
        }
    }
}

/// Suggestion types (Spec §8.2 Lines 13147-13158)
#[derive(Debug, Clone)]
pub enum Suggestion {
    /// Suggest wrapping in runtime check with Result type
    RuntimeCheck { code: Text, explanation: Text },
    /// Suggest adding compile-time proof/assertion
    CompileTimeProof { code: Text, explanation: Text },
    /// Suggest adding precondition to function
    Precondition { code: Text, explanation: Text },
    /// Suggest using a weaker type
    WeakenType { old_type: Text, new_type: Text },
    /// Custom suggestion with explanation
    Custom { message: Text, code: Maybe<Text> },
}

impl Suggestion {
    /// Format suggestion as help message
    pub fn format_help(&self) -> Text {
        match self {
            Suggestion::RuntimeCheck { code, explanation } => {
                Text::from(format!("{}\n\n  {}", explanation, code))
            }
            Suggestion::CompileTimeProof { code, explanation } => {
                Text::from(format!("{}\n\n  {}", explanation, code))
            }
            Suggestion::Precondition { code, explanation } => {
                Text::from(format!("{}\n\n  {}", explanation, code))
            }
            Suggestion::WeakenType { old_type, new_type } => Text::from(format!(
                "use weaker type `{}` instead of `{}`",
                new_type, old_type
            )),
            Suggestion::Custom { message, code } => match code {
                Maybe::Some(c) => Text::from(format!("{}\n\n  {}", message, c)),
                Maybe::None => message.clone(),
            },
        }
    }
}

/// Complete refinement diagnostic (Spec §8.2 Lines 13136-13144)
#[derive(Debug, Clone)]
pub struct RefinementDiagnostic {
    /// The actual value that failed (if known)
    pub actual_value: Maybe<ConstValue>,
    /// The constraint that was violated
    pub constraint: Text,
    /// Individual constraint evaluations (for compound predicates)
    pub constraint_evals: List<ConstraintEvaluation>,
    /// Error context
    pub context: ErrorContext,
    /// Source location
    pub span: Span,
    /// Actionable suggestions
    pub suggestions: List<Suggestion>,
    /// Additional notes
    pub notes: List<Text>,
}

impl RefinementDiagnostic {
    /// Create a new diagnostic
    pub fn new(constraint: Text, context: ErrorContext, span: Span) -> Self {
        Self {
            actual_value: Maybe::None,
            constraint,
            constraint_evals: List::new(),
            context,
            span,
            suggestions: List::new(),
            notes: List::new(),
        }
    }

    /// Set the actual value that failed
    pub fn with_value(mut self, value: ConstValue) -> Self {
        self.actual_value = Maybe::Some(value);
        self
    }

    /// Add a constraint evaluation
    pub fn add_constraint_eval(mut self, eval: ConstraintEvaluation) -> Self {
        self.constraint_evals.push(eval);
        self
    }

    /// Add a suggestion
    pub fn add_suggestion(mut self, suggestion: Suggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    /// Add a note
    pub fn add_note(mut self, note: Text) -> Self {
        self.notes.push(note);
        self
    }

    /// Format the complete error message (Spec §8.2 Lines 13397-13410)
    pub fn format_error(&self) -> Text {
        let mut output = Text::new();

        // Error header
        output.push_str("error: refinement constraint not satisfied\n");
        // Note: Span API changed - using byte offsets instead of line/col
        output.push_str(&format!(
            "  --> file_id:{} (bytes {}..{})\n",
            self.span.file_id.raw(),
            self.span.start,
            self.span.end
        ));
        output.push_str("   |\n");
        output.push_str(&format!("{:>4} | <source line>\n", self.span.start));

        // Value and constraint
        if let Maybe::Some(ref value) = self.actual_value {
            output.push_str(&format!(
                "   |    value `{}` fails constraint `{}`\n",
                value, self.constraint
            ));
        } else {
            output.push_str(&format!(
                "   |    value fails constraint `{}`\n",
                self.constraint
            ));
        }
        output.push_str("   |\n");

        // Type definition note
        output.push_str(&format!(
            "   = note: type `{}` is defined as `{}`\n",
            self.context.expected_type, self.context.expected_type
        ));

        // Constraint evaluations (Spec §8.3)
        if !self.constraint_evals.is_empty() {
            output.push_str("   = constraint evaluation:\n");
            for eval in &self.constraint_evals {
                output.push_str(&format!("     {}\n", eval.format_line()));
            }
            output.push_str("   |\n");
        }

        // Additional notes
        for note in &self.notes {
            output.push_str(&format!("   = note: {}\n", note));
        }

        // Suggestions
        for suggestion in &self.suggestions {
            output.push_str(&format!("   = help: {}\n", suggestion.format_help()));
        }

        output
    }
}

// ==================== Predicate Evaluation ====================

/// Evaluates predicates and decomposes compound expressions
pub struct PredicateEvaluator {
    /// Track whether we're in symbolic evaluation mode
    symbolic: bool,
}

impl PredicateEvaluator {
    pub fn new() -> Self {
        Self { symbolic: false }
    }

    /// Evaluate a predicate with a concrete value
    pub fn evaluate(
        &self,
        predicate: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> ConstraintResult {
        match self.evaluate_expr(predicate, value, var_name) {
            Ok(true) => ConstraintResult::Satisfied,
            Ok(false) => ConstraintResult::Violated,
            Err(_) => ConstraintResult::Unknown,
        }
    }

    /// Decompose compound predicates into individual constraints (Spec §8.3)
    pub fn decompose(&self, predicate: &Expr) -> List<ConstraintEvaluation> {
        let mut evals = List::new();
        self.decompose_recursive(predicate, &mut evals);
        evals
    }

    /// Decompose with actual value evaluation
    pub fn decompose_with_value(
        &self,
        predicate: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> List<ConstraintEvaluation> {
        let mut evals = List::new();
        self.decompose_recursive_eval(predicate, value, var_name, &mut evals);
        evals
    }

    // Private implementation methods

    fn evaluate_expr(&self, expr: &Expr, value: &ConstValue, var_name: &str) -> Result<bool, Text> {
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                self.evaluate_binary(op, left, right, value, var_name)
            }
            ExprKind::Unary { op, expr } => self.evaluate_unary(op, expr, value, var_name),
            ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::literal::LiteralKind::Bool(b) => Ok(*b),
                _ => Err("not a boolean".to_text()),
            },
            _ => Err("cannot evaluate".to_text()),
        }
    }

    fn evaluate_binary(
        &self,
        op: &BinOp,
        left: &Expr,
        right: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> Result<bool, Text> {
        match op {
            BinOp::And => {
                let l = self.evaluate_expr(left, value, var_name)?;
                let r = self.evaluate_expr(right, value, var_name)?;
                Ok(l && r)
            }
            BinOp::Or => {
                let l = self.evaluate_expr(left, value, var_name)?;
                let r = self.evaluate_expr(right, value, var_name)?;
                Ok(l || r)
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                // Try to evaluate comparison
                self.evaluate_comparison(op, left, right, value, var_name)
            }
            _ => Err("unsupported binary operation".to_text()),
        }
    }

    fn evaluate_unary(
        &self,
        op: &UnOp,
        expr: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> Result<bool, Text> {
        match op {
            UnOp::Not => {
                let val = self.evaluate_expr(expr, value, var_name)?;
                Ok(!val)
            }
            _ => Err("unsupported unary operation".to_text()),
        }
    }

    fn evaluate_comparison(
        &self,
        op: &BinOp,
        left: &Expr,
        right: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> Result<bool, Text> {
        // Get numeric values from both sides
        let left_val = self.get_numeric_value(left, value, var_name)?;
        let right_val = self.get_numeric_value(right, value, var_name)?;

        match op {
            BinOp::Eq => Ok(left_val == right_val),
            BinOp::Ne => Ok(left_val != right_val),
            BinOp::Lt => Ok(left_val < right_val),
            BinOp::Le => Ok(left_val <= right_val),
            BinOp::Gt => Ok(left_val > right_val),
            BinOp::Ge => Ok(left_val >= right_val),
            _ => Err("not a comparison".to_text()),
        }
    }

    fn get_numeric_value(
        &self,
        expr: &Expr,
        value: &ConstValue,
        var_name: &str,
    ) -> Result<i128, Text> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => Ok(int_lit.value),
                _ => Err("not an integer literal".to_text()),
            },
            ExprKind::Path(path) if path.is_single() => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    let name = ident.name.as_str();
                    if name == var_name || name == "it" {
                        value.as_i128().ok_or_else(|| "not an integer".to_text())
                    } else {
                        Err("unknown variable".to_text())
                    }
                } else {
                    Err("not a name".to_text())
                }
            }
            _ => Err("cannot get numeric value".to_text()),
        }
    }

    fn decompose_recursive(&self, expr: &Expr, evals: &mut List<ConstraintEvaluation>) {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => {
                // Recursively decompose AND expressions
                self.decompose_recursive(left, evals);
                self.decompose_recursive(right, evals);
            }
            _ => {
                // Atomic constraint
                let expr_str = self.format_expr(expr);
                evals.push(ConstraintEvaluation::new(
                    expr_str,
                    ConstraintResult::Unknown,
                ));
            }
        }
    }

    fn decompose_recursive_eval(
        &self,
        expr: &Expr,
        value: &ConstValue,
        var_name: &str,
        evals: &mut List<ConstraintEvaluation>,
    ) {
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::And,
                left,
                right,
            } => {
                // Recursively decompose AND expressions
                self.decompose_recursive_eval(left, value, var_name, evals);
                self.decompose_recursive_eval(right, value, var_name, evals);
            }
            _ => {
                // Atomic constraint - evaluate it
                let expr_str = self.format_expr(expr);
                let result = self.evaluate(expr, value, var_name);
                let explanation = self.explain_result(expr, value, var_name, &result);

                let mut eval = ConstraintEvaluation::new(expr_str, result);
                if let Some(exp) = explanation {
                    eval = eval.with_explanation(exp);
                }
                evals.push(eval);
            }
        }
    }

    fn explain_result(
        &self,
        expr: &Expr,
        value: &ConstValue,
        var_name: &str,
        result: &ConstraintResult,
    ) -> Option<Text> {
        if let ExprKind::Binary { op, left, right } = &expr.kind
            && let (Ok(lv), Ok(rv)) = (
                self.get_numeric_value(left, value, var_name),
                self.get_numeric_value(right, value, var_name),
            )
        {
            let op_str = match op {
                BinOp::Eq => "==",
                BinOp::Ne => "!=",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                _ => return None,
            };
            return Some(Text::from(format!(
                "{} {} {} = {}",
                lv,
                op_str,
                rv,
                if result.is_violated() {
                    "false"
                } else {
                    "true"
                }
            )));
        }
        None
    }

    fn format_expr(&self, expr: &Expr) -> Text {
        // Simplified expression formatting
        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let op_str = match op {
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    _ => "?",
                };
                Text::from(format!(
                    "{} {} {}",
                    self.format_expr(left),
                    op_str,
                    self.format_expr(right)
                ))
            }
            ExprKind::Unary { op, expr } => {
                let op_str = match op {
                    UnOp::Not => "!",
                    UnOp::Neg => "-",
                    _ => "?",
                };
                Text::from(format!("{}{}", op_str, self.format_expr(expr)))
            }
            ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => int_lit.value.to_text(),
                verum_ast::literal::LiteralKind::Bool(b) => b.to_text(),
                verum_ast::literal::LiteralKind::Text(s) => match s {
                    verum_ast::literal::StringLit::Regular(text) => {
                        Text::from(format!("\"{}\"", text))
                    }
                    _ => Text::from("?"),
                },
                _ => Text::from("?"),
            },
            ExprKind::Path(path) if path.is_single() => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    ident.name.to_text()
                } else {
                    Text::from("?")
                }
            }
            _ => Text::from("?"),
        }
    }
}

impl Default for PredicateEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Suggestion Generation ====================

/// Generates context-aware suggestions for fixing refinement errors
pub struct SuggestionGenerator;

impl SuggestionGenerator {
    /// Generate suggestions based on error context (Spec §8.2 Lines 13357-13385)
    pub fn generate(
        constraint: &str,
        context: &ErrorContext,
        _value: &Maybe<ConstValue>,
    ) -> List<Suggestion> {
        let mut suggestions = List::new();

        match context.refinement_source {
            RefinementSource::TypeAnnotation | RefinementSource::Assignment => {
                // Pattern 1: Direct assignment (Spec Lines 13357-13364)
                suggestions.push(Self::runtime_check_suggestion(constraint));
                suggestions.push(Self::try_from_suggestion(context.expected_type.as_ref()));
            }
            RefinementSource::FunctionParameter => {
                // Pattern 2: Function parameter (Spec Lines 13366-13374)
                suggestions.push(Self::validate_before_call(constraint));
                suggestions.push(Self::try_from_suggestion(context.expected_type.as_ref()));
                suggestions.push(Self::weaken_signature_suggestion(
                    context.expected_type.as_ref(),
                    context.actual_type.as_ref(),
                ));
            }
            RefinementSource::FunctionReturn => {
                // Pattern 3: Function return (Spec Lines 13376-13385)
                suggestions.push(Self::assert_before_return(constraint));
                suggestions.push(Self::result_type_suggestion(context.expected_type.as_ref()));
            }
            RefinementSource::FieldConstraint => {
                suggestions.push(Self::runtime_check_suggestion(constraint));
                suggestions.push(Self::validate_before_construction(constraint));
            }
        }

        suggestions
    }

    fn runtime_check_suggestion(constraint: &str) -> Suggestion {
        Suggestion::RuntimeCheck {
            code: format!(
                "if {} {{\n    // value satisfies constraint\n}} else {{\n    // handle constraint violation\n}}",
                constraint
            ).into(),
            explanation: "add a runtime check:".into(),
        }
    }

    fn try_from_suggestion(type_name: &str) -> Suggestion {
        Suggestion::RuntimeCheck {
            code: format!("let value = {}.try_from(value)?;", type_name).into(),
            explanation: "or use runtime validation:".into(),
        }
    }

    fn validate_before_call(constraint: &str) -> Suggestion {
        Suggestion::RuntimeCheck {
            code: format!(
                "if {} {{\n    func(value)\n}} else {{\n    // handle invalid input\n}}",
                constraint
            )
            .into(),
            explanation: "validate before calling:".into(),
        }
    }

    fn weaken_signature_suggestion(strong_type: &str, weak_type: &str) -> Suggestion {
        Suggestion::WeakenType {
            old_type: strong_type.into(),
            new_type: weak_type.into(),
        }
    }

    fn assert_before_return(constraint: &str) -> Suggestion {
        Suggestion::CompileTimeProof {
            code: format!("@assert {};\nvalue", constraint).into(),
            explanation: "add assertion before return:".into(),
        }
    }

    fn result_type_suggestion(type_name: &str) -> Suggestion {
        Suggestion::Custom {
            message: format!("change return type to `Result<{}>`", type_name).into(),
            code: Maybe::None,
        }
    }

    fn validate_before_construction(constraint: &str) -> Suggestion {
        Suggestion::RuntimeCheck {
            code: format!(
                "if {} {{\n    // construct with valid value\n}} else {{\n    return Err(\"invalid field value\")\n}}",
                constraint
            ).into(),
            explanation: "validate before construction:".into(),
        }
    }
}

// ==================== Diagnostic Builder ====================

/// Builder for creating RefinementDiagnostic instances
pub struct RefinementDiagnosticBuilder {
    constraint: Maybe<Text>,
    actual_value: Maybe<ConstValue>,
    context: Maybe<ErrorContext>,
    span: Maybe<Span>,
    predicate_expr: Maybe<Expr>,
    var_name: Maybe<Text>,
}

impl RefinementDiagnosticBuilder {
    pub fn new() -> Self {
        Self {
            constraint: Maybe::None,
            actual_value: Maybe::None,
            context: Maybe::None,
            span: Maybe::None,
            predicate_expr: Maybe::None,
            var_name: Maybe::None,
        }
    }

    pub fn constraint(mut self, constraint: Text) -> Self {
        self.constraint = Maybe::Some(constraint);
        self
    }

    pub fn actual_value(mut self, value: ConstValue) -> Self {
        self.actual_value = Maybe::Some(value);
        self
    }

    pub fn context(mut self, context: ErrorContext) -> Self {
        self.context = Maybe::Some(context);
        self
    }

    pub fn span(mut self, span: Span) -> Self {
        self.span = Maybe::Some(span);
        self
    }

    pub fn predicate_expr(mut self, expr: Expr) -> Self {
        self.predicate_expr = Maybe::Some(expr);
        self
    }

    pub fn var_name(mut self, name: Text) -> Self {
        self.var_name = Maybe::Some(name);
        self
    }

    pub fn build(self) -> RefinementDiagnostic {
        let constraint = self.constraint.unwrap_or_else(|| "unknown".into());
        let context = self.context.unwrap_or_else(|| ErrorContext {
            function_name: Maybe::None,
            expected_type: "Unknown".into(),
            actual_type: "Unknown".into(),
            refinement_source: RefinementSource::Assignment,
        });
        let span = self.span.unwrap_or_else(Span::dummy);

        let mut diagnostic = RefinementDiagnostic::new(constraint, context, span);

        if let Maybe::Some(value) = self.actual_value {
            diagnostic = diagnostic.with_value(value.clone());

            // Evaluate predicate if we have both value and expression
            if let (Maybe::Some(expr), Maybe::Some(var)) = (&self.predicate_expr, &self.var_name) {
                let evaluator = PredicateEvaluator::new();
                let evals = evaluator.decompose_with_value(expr, &value, var.as_ref());
                for eval in evals {
                    diagnostic = diagnostic.add_constraint_eval(eval);
                }
            }
        } else if let Maybe::Some(expr) = &self.predicate_expr {
            // No value - just decompose without evaluation
            let evaluator = PredicateEvaluator::new();
            let evals = evaluator.decompose(expr);
            for eval in evals {
                diagnostic = diagnostic.add_constraint_eval(eval);
            }
        }

        // Generate suggestions
        let suggestions = SuggestionGenerator::generate(
            diagnostic.constraint.as_ref(),
            &diagnostic.context,
            &diagnostic.actual_value,
        );
        for suggestion in suggestions {
            diagnostic = diagnostic.add_suggestion(suggestion);
        }

        diagnostic
    }
}

impl Default for RefinementDiagnosticBuilder {
    fn default() -> Self {
        Self::new()
    }
}
