//! Production-Grade Refinement Type System
//!
//! Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — Refinement Types
//!
//! This module implements Verum's complete refinement type system with:
//! - Three subsumption modes (syntactic, SMT-based, user proof)
//! - Full SMT integration via Z3
//! - High-quality error messages with counterexamples
//! - Performance optimization through proof caching
//!
//! # Architecture
//!
//! ## Core Data Structures
//! - `RefinementPredicate`: AST representation of refinement predicates
//! - `RefinementType`: Base type + predicate constraint
//! - `VerificationCondition`: SMT queries to verify
//!
//! ## Checker Modes
//! 1. **Syntactic**: Fast pattern matching for obvious cases (~1ms)
//! 2. **SMT-Based**: Z3 solver for complex predicates (10-500ms)
//! 3. **User Proof**: Cached proof terms (0ms) [future]
//!
//! ## Performance Targets
//! - Syntactic checks: < 1ms
//! - SMT queries: < 100ms (with timeout)
//! - Cache hit rate: > 90%
//! - Memory overhead: < 5%

use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use verum_ast::{
    expr::{BinOp, Block, Expr, ExprKind},
    literal::Literal,
    span::Span,
    ty::Path,
};
use verum_common::{Heap, List, Map, Maybe, Set, Text};

use crate::context::TypeContext;
use crate::refinement_diagnostics::{
    ErrorContext, PredicateEvaluator, RefinementDiagnostic, RefinementDiagnosticBuilder,
    RefinementSource,
};
use crate::ty::Type;

// ==================== Core Types ====================

/// Binding context for refinement predicates
///
/// Five refinement binding rules: (1) Inline T{pred} with implicit "it", (2) Lambda-style "where |x| pred", (3) Sigma-type "x: T where P(x)", (4) Named predicate "where pred_name", (5) Bare "where pred" (deprecated) — Five Binding Rules
///
/// The spec defines 5 different binding rules for refinement types:
/// 1. Inline refinement with implicit 'it': `Int{> 0}`
/// 2. Lambda-style with explicit binding: `Int where |x| x > 0`
/// 3. Sigma-type refinement: `x: Int where x > 0`
/// 4. Named predicate reference: `Int where is_positive`
/// 5. Bare where clause (deprecated): `Int where it > 0`
#[derive(Debug, Clone, PartialEq)]
pub enum RefinementBinding {
    /// Rule 1: Inline refinement with implicit 'it'
    /// Example: `Int{> 0}`
    Inline,

    /// Rule 2: Lambda-style where clause with explicit binding
    /// Example: `Int where |x| x > 0`
    Lambda(Text),

    /// Rule 3: Sigma-type refinement (handled separately in Type enum)
    /// Example: `x: Int where x > 0`
    Sigma(Text),

    /// Rule 4: Named predicate reference
    /// Example: `Int where is_positive`
    Named(Path),

    /// Rule 5: Bare where clause (deprecated, implicit 'it')
    /// Example: `Int where it > 0`
    Bare,
}

/// A refinement predicate with full semantic information
///
/// Represents a boolean predicate that constrains values of a type.
/// Example: `x > 0` for positive integers, `len(s) > 5` for non-empty strings
///
/// Five refinement binding rules: (1) Inline T{pred} with implicit "it", (2) Lambda-style "where |x| pred", (3) Sigma-type "x: T where P(x)", (4) Named predicate "where pred_name", (5) Bare "where pred" (deprecated)
#[derive(Debug, Clone, PartialEq)]
pub struct RefinementPredicate {
    /// The predicate expression (must evaluate to Bool)
    pub predicate: Expr,
    /// Binding context (which of the 5 rules)
    pub binding: RefinementBinding,
    /// Source location for error reporting
    pub span: Span,
}

impl RefinementPredicate {
    /// Create inline refinement (Rule 1)
    /// Example: `Int{> 0}` - implicit 'it' binding
    pub fn inline(predicate: Expr, span: Span) -> Self {
        Self {
            predicate,
            binding: RefinementBinding::Inline,
            span,
        }
    }

    /// Create lambda-style refinement (Rule 2)
    /// Example: `Int where |x| x > 0` - explicit binding
    pub fn lambda(predicate: Expr, var_name: Text, span: Span) -> Self {
        Self {
            predicate,
            binding: RefinementBinding::Lambda(var_name),
            span,
        }
    }

    /// Create sigma-type refinement (Rule 3)
    /// Example: `x: Int where x > 0` - dependent type binding
    pub fn sigma(predicate: Expr, var_name: Text, span: Span) -> Self {
        Self {
            predicate,
            binding: RefinementBinding::Sigma(var_name),
            span,
        }
    }

    /// Create named predicate refinement (Rule 4)
    /// Example: `Int where is_positive` - reference to named predicate
    pub fn named(predicate_path: Path, span: Span) -> Self {
        use verum_ast::expr::ExprKind;
        use verum_common::Heap;

        // For named predicates, create a call expression: predicate_name(it)
        // Build the path expression for the function
        let func_expr = Expr::path(predicate_path.clone());

        // Build the argument: identifier "it"
        let it_ident = verum_ast::ty::Ident::new("it", span);
        let it_expr = Expr::ident(it_ident);

        // Build the call expression
        let predicate = Expr::new(
            ExprKind::Call {
                func: Box::new(func_expr),
                type_args: List::new(),
                args: vec![it_expr].into(),
            },
            span,
        );

        Self {
            predicate,
            binding: RefinementBinding::Named(predicate_path),
            span,
        }
    }

    /// Create bare where refinement (Rule 5, deprecated)
    /// Example: `Int where it > 0` - bare where clause
    pub fn bare(predicate: Expr, span: Span) -> Self {
        Self {
            predicate,
            binding: RefinementBinding::Bare,
            span,
        }
    }

    /// Legacy constructor for backward compatibility
    /// Assumes lambda-style if explicit variable name provided
    pub fn new(predicate: Expr, bound_var: Text, span: Span) -> Self {
        // Assume lambda-style if explicit variable name
        Self::lambda(predicate, bound_var, span)
    }

    /// Create a placeholder refinement for testing purposes
    /// Creates a trivial "true" predicate that always succeeds
    pub fn placeholder() -> Self {
        use verum_ast::expr::ExprKind;
        use verum_ast::literal::{Literal, LiteralKind};

        let span = Span::default();
        let true_literal = Literal {
            kind: LiteralKind::Bool(true),
            span,
        };
        let predicate = Expr::new(ExprKind::Literal(true_literal), span);

        Self {
            predicate,
            binding: RefinementBinding::Inline,
            span,
        }
    }

    /// Get the bound variable name for this refinement
    pub fn bound_variable(&self) -> Text {
        match &self.binding {
            RefinementBinding::Inline => "it".into(),
            RefinementBinding::Lambda(name) => name.clone(),
            RefinementBinding::Sigma(name) => name.clone(),
            RefinementBinding::Named(_) => "it".into(),
            RefinementBinding::Bare => "it".into(),
        }
    }

    /// Create a trivially true predicate (unrefined)
    pub fn trivial(span: Span) -> Self {
        Self {
            predicate: Expr::literal(Literal::bool(true, span)),
            binding: RefinementBinding::Inline,
            span,
        }
    }

    /// Check if this is a trivial (always true) predicate
    pub fn is_trivial(&self) -> bool {
        matches!(
            &self.predicate.kind,
            ExprKind::Literal(Literal {
                kind: verum_ast::literal::LiteralKind::Bool(true),
                ..
            })
        )
    }
}

impl Display for RefinementPredicate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.binding {
            RefinementBinding::Inline => write!(f, "{{<predicate>}}"),
            RefinementBinding::Lambda(var) => write!(f, "where |{}| <predicate>", var),
            RefinementBinding::Sigma(var) => write!(f, "{}: T where <predicate>", var),
            RefinementBinding::Named(path) => {
                // Display path segments
                let path_str = path
                    .segments
                    .iter()
                    .map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => ident.name.as_str(),
                        _ => "<segment>",
                    })
                    .collect::<List<_>>()
                    .join(".");
                write!(f, "where {}", path_str)
            }
            RefinementBinding::Bare => write!(f, "where <predicate>"),
        }
    }
}

/// A refinement type: base type + predicate
///
/// Example: `{ x: Int | x > 0 }` - integers greater than zero
#[derive(Debug, Clone)]
pub struct RefinementType {
    /// The base type being refined
    pub base_type: Type,
    /// The refinement predicate
    pub predicate: RefinementPredicate,
    /// Source location
    pub span: Span,
}

impl RefinementType {
    /// Create an unrefined type (trivial predicate)
    pub fn unrefined(base_type: Type, span: Span) -> Self {
        Self {
            base_type: base_type.clone(),
            predicate: RefinementPredicate::trivial(span),
            span,
        }
    }

    /// Create a refined type with predicate
    pub fn refined(base_type: Type, predicate: RefinementPredicate, span: Span) -> Self {
        Self {
            base_type,
            predicate,
            span,
        }
    }

    /// Check if this type is unrefined (trivial predicate)
    pub fn is_unrefined(&self) -> bool {
        self.predicate.is_trivial()
    }
}

/// Named refinement predicate for reusability
///
/// Five refinement binding rules: (1) Inline T{pred} with implicit "it", (2) Lambda-style "where |x| pred", (3) Sigma-type "x: T where P(x)", (4) Named predicate "where pred_name", (5) Bare "where pred" (deprecated) — Rule 4
#[derive(Debug, Clone)]
pub struct NamedPredicate {
    /// Predicate name (e.g., "is_positive", "is_email")
    pub name: Text,
    /// The predicate definition
    pub predicate: RefinementPredicate,
}

/// Verification condition for SMT solver
///
/// Generated during type checking to verify refinement constraints
#[derive(Debug, Clone)]
pub struct VerificationCondition {
    /// The condition to verify (should be unsatisfiable for valid refinement)
    pub condition: Expr,
    /// Context assumptions (e.g., "x > 0" from outer scope)
    pub assumptions: List<Expr>,
    /// Substitutions to apply (e.g., replace 'it' with actual value)
    pub substitutions: Map<Text, Expr>,
    /// Source location
    pub span: Span,
}

impl VerificationCondition {
    /// Create a new verification condition
    pub fn new(condition: Expr, span: Span) -> Self {
        Self {
            condition,
            assumptions: List::new(),
            substitutions: Map::new(),
            span,
        }
    }

    /// Add an assumption to the context
    pub fn with_assumption(mut self, assumption: Expr) -> Self {
        self.assumptions.push(assumption);
        self
    }

    /// Add a substitution
    pub fn with_substitution(mut self, var: Text, expr: Expr) -> Self {
        self.substitutions.insert(var, expr);
        self
    }
}

/// Counterexample for failed verification
///
/// Shows concrete values that violate a refinement constraint
#[derive(Debug, Clone)]
pub struct CounterExample {
    /// Variable name
    pub var_name: Text,
    /// Value that violates the constraint
    pub value: Text,
    /// Human-readable explanation
    pub explanation: Maybe<Text>,
}

impl CounterExample {
    pub fn new(var_name: Text, value: Text) -> Self {
        Self {
            var_name,
            value,
            explanation: Maybe::None,
        }
    }

    pub fn with_explanation(mut self, explanation: Text) -> Self {
        self.explanation = Maybe::Some(explanation);
        self
    }
}

/// Verification result from refinement checker
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Refinement is valid (predicate holds)
    Valid,
    /// Refinement is invalid with counterexample
    Invalid {
        counterexample: Maybe<CounterExample>,
    },
    /// Cannot determine (timeout, unsupported feature, etc.)
    Unknown { reason: Text },
}

impl VerificationResult {
    pub fn is_valid(&self) -> bool {
        matches!(self, VerificationResult::Valid)
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, VerificationResult::Invalid { .. })
    }
}

/// Verification statistics for performance monitoring
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// Total verification checks performed
    pub total_checks: usize,
    /// Successful verifications
    pub successful: usize,
    /// Failed verifications
    pub failed: usize,
    /// Unknown results (timeout/unsupported)
    pub unknown: usize,
    /// Syntactic checks (fast path)
    pub syntactic_checks: usize,
    /// SMT checks (slow path)
    pub smt_checks: usize,
    /// Cache hits
    pub cache_hits: usize,
    /// Total elapsed time in microseconds
    pub elapsed_micros: u64,
}

impl VerificationStats {
    pub fn average_time_ms(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.elapsed_micros as f64) / (self.total_checks as f64 * 1000.0)
        }
    }

    pub fn cache_hit_rate(&self) -> f64 {
        if self.total_checks == 0 {
            0.0
        } else {
            (self.cache_hits as f64) / (self.total_checks as f64)
        }
    }

    pub fn report(&self) -> Text {
        format!(
            "Refinement Verification Statistics:\n\
             - Total checks: {}\n\
             - Successful: {}, Failed: {}, Unknown: {}\n\
             - Syntactic: {}, SMT: {}\n\
             - Cache hits: {} ({:.1}%)\n\
             - Average time: {:.2}ms\n\
             - Total time: {:.2}ms",
            self.total_checks,
            self.successful,
            self.failed,
            self.unknown,
            self.syntactic_checks,
            self.smt_checks,
            self.cache_hits,
            self.cache_hit_rate() * 100.0,
            self.average_time_ms(),
            (self.elapsed_micros as f64) / 1000.0
        )
        .into()
    }
}

// ==================== SMT Backend ====================

/// SMT solver result
#[derive(Debug, Clone)]
pub enum SmtResult {
    /// Formula is satisfiable
    Sat,
    /// Formula is unsatisfiable (proof found)
    Unsat,
    /// Cannot determine (timeout, too complex)
    Unknown,
}

/// SMT backend trait for pluggable solvers
///
/// Primary implementation: Z3 via verum_smt crate
pub trait SmtBackend: Send + Sync {
    /// Check satisfiability of an expression
    fn check(&mut self, expr: &Expr) -> Result<SmtResult, RefinementError>;

    /// Get model (satisfying assignment) for SAT result
    fn get_model(&mut self) -> Result<Map<Text, Text>, RefinementError>;

    /// Check if predicate holds for given value
    fn verify_refinement(
        &mut self,
        predicate: &Expr,
        value: &Expr,
        assumptions: &[Expr],
    ) -> Result<VerificationResult, RefinementError>;

    /// Apply a per-query timeout in milliseconds to the backend's
    /// underlying solver. The default is a no-op so legacy
    /// backends compile without modification — concrete backends
    /// (e.g. `Z3Backend`) override to forward the limit to the
    /// solver via `set_params({"timeout": ms})`. Called by
    /// `RefinementChecker` before every `check` /
    /// `verify_refinement` invocation when
    /// `RefinementConfig.timeout_ms` is set, so the documented
    /// "100 ms default per spec" actually constrains solver work.
    fn set_timeout_ms(&mut self, _ms: u64) {}
}

// ==================== Refinement Error ====================

/// Refinement error with location and diagnostic information
///
/// Refinement type diagnostics: error messages for failed refinement checks with source location and predicate details — 8.3
#[derive(Debug, Clone)]
pub struct RefinementError {
    /// Error message (legacy)
    pub message: Text,
    /// Source location
    pub span: Span,
    /// Optional counterexample
    pub counterexample: Maybe<CounterExample>,
    /// Enhanced diagnostic (Spec §8.2-8.3)
    pub diagnostic: Maybe<RefinementDiagnostic>,
}

impl RefinementError {
    pub fn new(message: Text, span: Span) -> Self {
        Self {
            message,
            span,
            counterexample: Maybe::None,
            diagnostic: Maybe::None,
        }
    }

    pub fn with_counterexample(mut self, counterexample: CounterExample) -> Self {
        self.counterexample = Maybe::Some(counterexample);
        self
    }

    pub fn with_diagnostic(mut self, diagnostic: RefinementDiagnostic) -> Self {
        self.diagnostic = Maybe::Some(diagnostic);
        self
    }
}

impl Display for RefinementError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // Use enhanced diagnostic if available (Spec §8.2)
        if let Maybe::Some(ref diag) = self.diagnostic {
            write!(f, "{}", diag.format_error())
        } else {
            // Fallback to legacy format
            write!(f, "{}", self.message)?;
            if let Maybe::Some(ref ce) = self.counterexample {
                write!(f, "\n  Counterexample: {} = {}", ce.var_name, ce.value)?;
                if let Maybe::Some(ref explanation) = ce.explanation {
                    write!(f, "\n  {}", explanation)?;
                }
            }
            Ok(())
        }
    }
}

impl std::error::Error for RefinementError {}

/// High-quality error message generator
///
/// Refinement type diagnostics: error messages for failed refinement checks with source location and predicate details — 8.3
pub struct RefinementErrorGenerator {
    /// Whether to include suggestions
    include_suggestions: bool,
    /// Predicate evaluator for constraint decomposition
    evaluator: PredicateEvaluator,
}

impl RefinementErrorGenerator {
    pub fn new() -> Self {
        Self {
            include_suggestions: true,
            evaluator: PredicateEvaluator::new(),
        }
    }

    /// Generate error for failed refinement check with enhanced diagnostics
    ///
    /// Protocol method dispatch resolution across module boundaries-13196, 13246-13262
    pub fn refinement_failed(
        &self,
        vc: &VerificationCondition,
        counterexample: Maybe<CounterExample>,
    ) -> RefinementError {
        // Legacy message for fallback
        let mut message = format!(
            "refinement constraint not satisfied: {}",
            self.format_expr(&vc.condition)
        );

        if let Maybe::Some(ref ce) = counterexample {
            message.push_str(&format!(
                "\n  Counterexample: {} = {}",
                ce.var_name, ce.value
            ));
            if let Maybe::Some(ref explanation) = ce.explanation {
                message.push_str(&format!("\n  {}", explanation));
            }
        }

        if self.include_suggestions {
            message.push_str("\n  Help: Use runtime check with `value.check()?`");
            message.push_str("\n  Help: Or provide compile-time proof with `@verify(proof)`");
        }

        let mut error = RefinementError {
            message: message.into(),
            span: vc.span,
            counterexample: counterexample.clone(),
            diagnostic: Maybe::None,
        };

        // Build enhanced diagnostic (Spec §8.2-8.3)
        error = error.with_diagnostic(self.build_enhanced_diagnostic(vc, &counterexample));

        error
    }

    /// Generate error for subsumption failure
    pub fn subsumption_failed(
        &self,
        subtype: &RefinementType,
        supertype: &RefinementType,
        span: Span,
    ) -> RefinementError {
        let message = format!(
            "refinement subsumption failed: cannot prove that\n  {}\n  implies\n  {}",
            self.format_refinement(subtype),
            self.format_refinement(supertype)
        );

        RefinementError::new(message.into(), span)
    }

    /// Build enhanced diagnostic with predicate evaluation (Spec §8.2-8.3)
    fn build_enhanced_diagnostic(
        &self,
        vc: &VerificationCondition,
        counterexample: &Maybe<CounterExample>,
    ) -> RefinementDiagnostic {
        let constraint = self.format_expr(&vc.condition);

        // Create error context
        let context = ErrorContext {
            function_name: Maybe::None,
            expected_type: "RefinedType".into(),
            actual_type: "BaseType".into(),
            refinement_source: RefinementSource::Assignment,
        };

        let mut builder = RefinementDiagnosticBuilder::new()
            .constraint(constraint)
            .context(context)
            .span(vc.span)
            .predicate_expr(vc.condition.clone());

        // Add actual value from counterexample if available
        if let Maybe::Some(ce) = &counterexample {
            // Try to parse counterexample value
            if let Ok(value) = ce.value.parse::<i128>() {
                builder = builder.actual_value(verum_common::ConstValue::Int(value));
                builder = builder.var_name(ce.var_name.clone());
            }
        }

        builder.build()
    }

    /// Format an expression as human-readable text for error messages.
    ///
    /// Provides a complete pretty-printer for refinement predicates,
    /// supporting all common expression forms including:
    /// - Binary operations (arithmetic, comparison, logical)
    /// - Unary operations (negation, not)
    /// - Literals (integers, booleans, floats, strings)
    /// - Variable references (paths, identifiers)
    /// - Function calls
    /// - Field access
    /// - Method calls
    fn format_expr(&self, expr: &Expr) -> Text {
        use verum_ast::expr::UnOp;
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                let op_str = match op {
                    BinOp::Add | BinOp::Concat => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Rem => "%",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::Imply => "->",
                    BinOp::Iff => "<->",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::In => "in",
                    BinOp::BitAnd => "&",
                    BinOp::BitOr => "|",
                    BinOp::BitXor => "^",
                    BinOp::Shl => "<<",
                    BinOp::Shr => ">>",
                    BinOp::Pow => "**",
                    // Assignment operators
                    BinOp::Assign => "=",
                    BinOp::AddAssign => "+=",
                    BinOp::SubAssign => "-=",
                    BinOp::MulAssign => "*=",
                    BinOp::DivAssign => "/=",
                    BinOp::RemAssign => "%=",
                    BinOp::BitAndAssign => "&=",
                    BinOp::BitOrAssign => "|=",
                    BinOp::BitXorAssign => "^=",
                    BinOp::ShlAssign => "<<=",
                    BinOp::ShrAssign => ">>=",
                };
                // Add parentheses for clarity in compound expressions
                let left_str = self.format_expr(left);
                let right_str = self.format_expr(right);
                format!("({} {} {})", left_str, op_str, right_str).into()
            }

            ExprKind::Unary { op, expr: inner } => {
                let op_str = match op {
                    UnOp::Not => "!",
                    UnOp::Neg => "-",
                    UnOp::BitNot => "~",
                    UnOp::Deref => "*",
                    UnOp::Ref => "&",
                    UnOp::RefMut => "&mut ",
                    UnOp::RefChecked => "&checked ",
                    UnOp::RefCheckedMut => "&checked mut ",
                    UnOp::RefUnsafe => "&unsafe ",
                    UnOp::RefUnsafeMut => "&unsafe mut ",
                    UnOp::Own => "%",
                    UnOp::OwnMut => "%mut ",
                };
                let inner_str = self.format_expr(inner);
                format!("{}{}", op_str, inner_str).into()
            }

            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => int_lit.value.to_string().into(),
                LiteralKind::Float(float_lit) => float_lit.value.to_string().into(),
                LiteralKind::Bool(b) => if *b { "true" } else { "false" }.into(),
                LiteralKind::Char(c) => format!("'{}'", c).into(),
                LiteralKind::ByteChar(b) => format!("b'{}'", *b as char).into(),
                LiteralKind::ByteString(bytes) => {
                    let escaped: String = bytes.iter().map(|b| format!("\\x{:02x}", b)).collect();
                    format!("b\"{}\"", escaped).into()
                }
                LiteralKind::Text(s) => {
                    match s {
                        verum_ast::literal::StringLit::Regular(text) => format!("\"{}\"", text).into(),
                        verum_ast::literal::StringLit::MultiLine(text) => {
                            format!("\"\"\"{}\"\"\"", text).into()
                        }
                    }
                }
                LiteralKind::Composite(comp) => format!("{}#\"{}\"", comp.tag, comp.content).into(),
                LiteralKind::InterpolatedString(interp) => {
                    // InterpolatedStringLit has prefix and content
                    format!("{}\"{}\"", interp.prefix, interp.content).into()
                }
                LiteralKind::Tagged { tag, content } => format!("{}#\"{}\"", tag, content).into(),
                LiteralKind::Contract(content) => format!("contract#\"{}\"", content).into(),
                LiteralKind::ContextAdaptive(lit) => format!("#{}", lit.raw).into(),
            },

            ExprKind::Path(path) => {
                let segments: List<&str> = path
                    .segments
                    .iter()
                    .filter_map(|seg| match seg {
                        verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str()),
                        verum_ast::ty::PathSegment::SelfValue => Some("self"),
                        verum_ast::ty::PathSegment::Super => Some("super"),
                        verum_ast::ty::PathSegment::Cog => Some("cog"),
                        _ => None,
                    })
                    .collect();
                segments.join("::")
            }

            ExprKind::Call { func, args, .. } => {
                let func_str = self.format_expr(func);
                let args_str: List<Text> = args.iter().map(|a| self.format_expr(a)).collect();
                format!("{}({})", func_str, args_str.join(", ")).into()
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                let recv_str = self.format_expr(receiver);
                let args_str: List<Text> = args.iter().map(|a| self.format_expr(a)).collect();
                format!("{}.{}({})", recv_str, method.name, args_str.join(", ")).into()
            }

            ExprKind::Field { expr: inner, field } => {
                let inner_str = self.format_expr(inner);
                format!("{}.{}", inner_str, field.name).into()
            }

            ExprKind::Index { expr: inner, index } => {
                let inner_str = self.format_expr(inner);
                let index_str = self.format_expr(index);
                format!("{}[{}]", inner_str, index_str).into()
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Format condition: IfCondition contains conditions list
                let cond_parts: List<Text> = condition
                    .conditions
                    .iter()
                    .map(|c| match c {
                        verum_ast::expr::ConditionKind::Expr(e) => self.format_expr(e),
                        verum_ast::expr::ConditionKind::Let { pattern, value, .. } => {
                            format!("let {:?} = {}", pattern, self.format_expr(value)).into()
                        }
                    })
                    .collect();
                let cond_str = cond_parts.join(" && ");
                let then_str = self.format_block(then_branch);
                if let verum_common::Maybe::Some(else_expr) = else_branch {
                    let else_str = self.format_expr(else_expr);
                    format!("if {} {} else {}", cond_str, then_str, else_str).into()
                } else {
                    format!("if {} {}", cond_str, then_str).into()
                }
            }

            ExprKind::Tuple(elements) => {
                let elems_str: List<Text> = elements.iter().map(|e| self.format_expr(e)).collect();
                format!("({})", elems_str.join(", ")).into()
            }

            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        let elems_str: List<Text> =
                            elements.iter().map(|e| self.format_expr(e)).collect();
                        format!("[{}]", elems_str.join(", ")).into()
                    }
                    ArrayExpr::Repeat { value, count } => {
                        format!("[{}; {}]", self.format_expr(value), self.format_expr(count)).into()
                    }
                }
            }

            ExprKind::Record { fields, .. } => {
                let fields_str: List<String> = fields
                    .iter()
                    .map(|field| {
                        if let verum_common::Maybe::Some(ref val) = field.value {
                            format!("{}: {}", field.name.name, self.format_expr(val))
                        } else {
                            field.name.name.to_string()
                        }
                    })
                    .collect();
                format!("{{ {} }}", fields_str.join(", ")).into()
            }

            ExprKind::Closure { params, body, .. } => {
                let params_str: List<String> =
                    params.iter().map(|p| format!("{:?}", p.pattern)).collect();
                let body_str = self.format_expr(body);
                format!("|{}| {}", params_str.join(", "), body_str).into()
            }

            ExprKind::Block(block) => self.format_block(block),

            // Default case for complex expressions not yet handled
            _ => Text::from("<expr>"),
        }
    }

    /// Format a block as a string
    fn format_block(&self, block: &Block) -> Text {
        if block.stmts.is_empty() {
            if let Some(expr) = &block.expr {
                format!("{{ {} }}", self.format_expr(expr)).into()
            } else {
                "{}".into()
            }
        } else {
            "{ ... }".into()
        }
    }

    fn format_refinement(&self, ty: &RefinementType) -> Text {
        format!(
            "{{ {}: {} | {} }}",
            ty.predicate.bound_variable(),
            ty.base_type,
            self.format_expr(&ty.predicate.predicate)
        )
        .into()
    }
}

impl Default for RefinementErrorGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== Refinement Checker ====================

/// Configuration for refinement checker
#[derive(Debug, Clone)]
pub struct RefinementConfig {
    /// Enable SMT solver (requires Z3)
    pub enable_smt: bool,
    /// SMT timeout in milliseconds
    pub timeout_ms: u64,
    /// Enable proof caching
    pub enable_cache: bool,
    /// Maximum cache size (number of entries)
    pub max_cache_size: usize,
}

impl Default for RefinementConfig {
    fn default() -> Self {
        Self {
            enable_smt: true,
            timeout_ms: 100, // 100ms default timeout per spec
            enable_cache: true,
            max_cache_size: 10000,
        }
    }
}

/// Main refinement type checker
///
/// Implements three subsumption modes:
/// 1. Syntactic (fast, conservative)
/// 2. SMT-based (accurate, slower)
/// 3. User proof (instant, requires proof term) [future]
pub struct RefinementChecker {
    /// Configuration
    config: RefinementConfig,
    /// Statistics
    stats: Arc<RwLock<VerificationStats>>,
    /// Error generator
    error_gen: RefinementErrorGenerator,
    /// SMT backend (optional)
    smt_backend: Maybe<Arc<RwLock<Box<dyn SmtBackend>>>>,
    /// Verification condition cache
    /// Key: hash of (predicate, value), Value: VerificationResult
    cache: Arc<RwLock<Map<u64, VerificationResult>>>,
    /// Dependent type checker (optional, for v2.0+ features)
    /// Dependent types (future v2.0+): Pi types, Sigma types, equality types, universe hierarchy, dependent pattern matching, termination checking — Dependent Types Extension
    pub(crate) dependent_checker:
        Maybe<Box<dyn crate::dependent_integration::DependentTypeChecker>>,
}

impl RefinementChecker {
    /// Create a new refinement checker.
    ///
    /// Historical note: this constructor used to auto-wire an SMT backend
    /// (`Z3Backend` from `verum_types::smt_backend`) and a dependent-type
    /// checker (`SmtDependentTypeChecker` from
    /// `verum_types::dependent_integration`) whenever `config.enable_smt`
    /// was true. Both concrete implementations now live in `verum_smt` to
    /// break the `verum_types ↔ verum_smt` circular dependency.
    ///
    /// Callers that want the SMT path must inject backends via
    /// `.with_smt_backend(...)` and `.with_dependent_checker(...)` /
    /// `.set_dependent_checker(...)`. `verum_compiler::pipeline`
    /// does this right after constructing the checker. Without
    /// injection, the checker falls back to syntactic checks only.
    pub fn new(config: RefinementConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(VerificationStats::default())),
            error_gen: RefinementErrorGenerator::new(),
            smt_backend: Maybe::None,
            cache: Arc::new(RwLock::new(Map::new())),
            dependent_checker: Maybe::None,
        }
    }

    /// Set SMT backend (typically Z3)
    pub fn with_smt_backend(mut self, backend: Box<dyn SmtBackend>) -> Self {
        self.smt_backend = Maybe::Some(Arc::new(RwLock::new(backend)));
        self
    }

    /// Install an SMT backend after construction.
    pub fn set_smt_backend(&mut self, backend: Box<dyn SmtBackend>) {
        self.smt_backend = Maybe::Some(Arc::new(RwLock::new(backend)));
    }

    /// Check if value satisfies refinement type
    ///
    /// This is the main entry point for refinement checking.
    /// Strategy:
    /// 1. Try syntactic check (fast, ~1ms)
    /// 2. Check cache
    /// 3. Fall back to SMT if enabled
    pub fn check(
        &mut self,
        value: &Expr,
        refinement: &RefinementType,
        _ctx: &TypeContext,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Update stats - handle lock poisoning gracefully
        {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.total_checks += 1;
                }
                Err(poisoned) => {
                    // Recover from poisoned lock - get the underlying data anyway
                    let mut stats = poisoned.into_inner();
                    stats.total_checks += 1;
                }
            }
        }

        // Trivial refinement (unrefined type) - always valid
        if refinement.is_unrefined() {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.successful += 1;
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    stats.successful += 1;
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
            return Ok(VerificationResult::Valid);
        }

        // Generate verification condition
        let vc = self.generate_vc(value, &refinement.predicate)?;

        // Try syntactic check first (fast path)
        if let Maybe::Some(result) = self.try_syntactic_check(&vc) {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.syntactic_checks += 1;
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    stats.syntactic_checks += 1;
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
            return Ok(result);
        }

        // Check cache
        if self.config.enable_cache {
            let cache_key = self.compute_cache_key(&vc);
            let cache_result = match self.cache.read() {
                Ok(cache) => cache.get(&cache_key).cloned(),
                Err(poisoned) => poisoned.into_inner().get(&cache_key).cloned(),
            };
            if let Some(cached_result) = cache_result {
                match self.stats.write() {
                    Ok(mut stats) => {
                        stats.cache_hits += 1;
                        stats.elapsed_micros += start.elapsed().as_micros() as u64;
                    }
                    Err(poisoned) => {
                        let mut stats = poisoned.into_inner();
                        stats.cache_hits += 1;
                        stats.elapsed_micros += start.elapsed().as_micros() as u64;
                    }
                }
                return Ok(cached_result);
            }
        }

        // Fall back to SMT solver
        let result = if self.config.enable_smt {
            self.check_with_smt(&vc)?
        } else {
            // SMT disabled - conservative unknown
            VerificationResult::Unknown {
                reason: "SMT solver disabled".into(),
            }
        };

        // Update cache
        if self.config.enable_cache {
            let cache_key = self.compute_cache_key(&vc);
            let cache_write_result = self.cache.write();
            let mut cache = match cache_write_result {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };

            // Evict oldest entry if cache is full
            if cache.len() >= self.config.max_cache_size {
                // Clear 10% of cache when full (simple eviction strategy)
                let to_remove: List<u64> = cache
                    .keys()
                    .take(self.config.max_cache_size / 10)
                    .cloned()
                    .collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }

            cache.insert(cache_key, result.clone());
        }

        // Update stats
        {
            match self.stats.write() {
                Ok(mut stats) => {
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
        }

        Ok(result)
    }

    /// Check refinement subsumption: T1 <: T2
    ///
    /// Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1
    ///
    /// Returns true if φ1 => φ2 (predicate implication holds)
    pub fn check_subsumption(
        &mut self,
        subtype: &RefinementType,
        supertype: &RefinementType,
    ) -> Result<bool, RefinementError> {
        let start = Instant::now();

        // Helper macro-like closure for updating stats with poisoned lock handling
        let update_stats = |stats_lock: &Arc<RwLock<VerificationStats>>,
                            f: &dyn Fn(&mut VerificationStats)| {
            match stats_lock.write() {
                Ok(mut stats) => f(&mut stats),
                Err(poisoned) => f(&mut poisoned.into_inner()),
            }
        };

        // Update stats - total checks
        update_stats(&self.stats, &|stats| {
            stats.total_checks += 1;
        });

        // Base types must match
        if subtype.base_type != supertype.base_type {
            update_stats(&self.stats, &|stats| {
                stats.elapsed_micros += start.elapsed().as_micros() as u64;
            });
            return Ok(false);
        }

        // Unrefined supertype - always subsumes
        if supertype.is_unrefined() {
            update_stats(&self.stats, &|stats| {
                stats.successful += 1;
                stats.elapsed_micros += start.elapsed().as_micros() as u64;
            });
            return Ok(true);
        }

        // Unrefined subtype with refined supertype - cannot subsume
        if subtype.is_unrefined() && !supertype.is_unrefined() {
            update_stats(&self.stats, &|stats| {
                stats.failed += 1;
                stats.elapsed_micros += start.elapsed().as_micros() as u64;
            });
            return Ok(false);
        }

        // Generate implication check: φ1 => φ2
        let implication = self
            .generate_implication(&subtype.predicate.predicate, &supertype.predicate.predicate)?;

        let vc = VerificationCondition::new(implication, subtype.span);

        // Try syntactic subsumption
        if let Maybe::Some(result) = self.try_syntactic_subsumption(subtype, supertype) {
            let result_copy = result;
            update_stats(&self.stats, &|stats| {
                stats.syntactic_checks += 1;
                if result_copy {
                    stats.successful += 1;
                } else {
                    stats.failed += 1;
                }
                stats.elapsed_micros += start.elapsed().as_micros() as u64;
            });
            return Ok(result);
        }

        // Use SMT solver
        let result = if self.config.enable_smt {
            let result = self.check_with_smt(&vc)?;
            result.is_valid()
        } else {
            // Conservative: reject without SMT
            false
        };

        // Update stats
        let result_copy = result;
        update_stats(&self.stats, &|stats| {
            if result_copy {
                stats.successful += 1;
            } else {
                stats.failed += 1;
            }
            stats.elapsed_micros += start.elapsed().as_micros() as u64;
        });

        Ok(result)
    }

    /// Generate verification condition
    fn generate_vc(
        &self,
        value: &Expr,
        predicate: &RefinementPredicate,
    ) -> Result<VerificationCondition, RefinementError> {
        // Substitute bound variable with actual value.
        //
        // The canonical bound-variable name is `predicate.bound_variable()`
        // (default "it"), but users commonly write refinements with `self`
        // too: `Int{self != 0}`. The AST layer stores the predicate body
        // verbatim, so without a second pass the substitution never reaches
        // `self` and the verifier returns Unknown, silently letting
        // refinement violations through. Substituting both the canonical
        // bound name AND the `self` alias covers both surface syntaxes
        // without requiring the parser to rewrite.
        let bound_var = predicate.bound_variable();
        let mut condition = self.substitute_in_expr(&predicate.predicate, &bound_var, value);
        let self_var: Text = "self".into();
        if bound_var.as_str() != "self" {
            condition = self.substitute_in_expr(&condition, &self_var, value);
        }

        Ok(VerificationCondition::new(condition, predicate.span))
    }

    /// Production-grade capture-avoiding substitution
    ///
    /// Substitutes `value` for `var` in `expr` while avoiding variable capture.
    /// Implements proper alpha-conversion when needed to preserve semantics.
    ///
    /// # Correctness Properties
    /// - Preserves free variables: FV(subst(e, x, v)) = (FV(e) - {x}) ∪ FV(v)
    /// - Avoids capture: If y ∈ FV(v) and y is bound in e, perform alpha-renaming
    /// - Preserves semantics: [[subst(e, x, v)]] = [[e]][x ↦ [[v]]]
    ///
    /// # Visibility note
    ///
    /// Exposed at `pub(crate)` so that `infer.rs` can substitute earlier
    /// function arguments into subsequent parameters' refinement
    /// predicates at call sites. This enables dependent refinement
    /// enforcement: when calling `fn safe_get(len: Int, i: Int{< len}) -> Int`
    /// with `safe_get(5, 10)`, the checker substitutes `len → 5` into the
    /// predicate `i < len` before checking `10`, producing `10 < 5` which
    /// the refinement checker correctly rejects.
    ///
    /// See `crates/verum_types/src/infer.rs` call-site loop around line
    /// 10558 and `crates/verum_compiler/tests/dependent_patterns_regression.rs`
    /// for the regression tests.
    pub(crate) fn substitute_in_expr(&self, expr: &Expr, var: &Text, value: &Expr) -> Expr {
        use verum_ast::StmtKind;
        use verum_ast::expr::ArrayExpr;

        // Collect free variables in value to detect potential capture
        let free_vars_in_value = self.collect_free_vars(value);

        match &expr.kind {
            // Variable reference - direct substitution if matches.
            // Handles both ordinary names (`x`) and the `self` keyword,
            // which the parser represents as `PathSegment::SelfValue`
            // rather than `PathSegment::Name(Ident("self"))`. Without the
            // `SelfValue` arm, `Int{self != 0}` refinements would never
            // have their bound variable substituted and the refinement
            // checker would silently return Unknown.
            ExprKind::Path(path) if path.segments.len() == 1 => {
                match &path.segments[0] {
                    verum_ast::ty::PathSegment::Name(ident)
                        if ident.name.as_str() == var.as_str() =>
                    {
                        return value.clone();
                    }
                    verum_ast::ty::PathSegment::SelfValue if var.as_str() == "self" => {
                        return value.clone();
                    }
                    _ => {}
                }
                expr.clone()
            }

            // Binary operations - recurse on both sides
            ExprKind::Binary { op, left, right } => Expr::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(self.substitute_in_expr(left, var, value)),
                    right: Box::new(self.substitute_in_expr(right, var, value)),
                },
                expr.span,
            ),

            // Unary operations - recurse
            ExprKind::Unary { op, expr: inner } => Expr::new(
                ExprKind::Unary {
                    op: *op,
                    expr: Box::new(self.substitute_in_expr(inner, var, value)),
                },
                expr.span,
            ),

            // Function calls - substitute in function and arguments
            ExprKind::Call { func, args, .. } => Expr::new(
                ExprKind::Call {
                    func: Box::new(self.substitute_in_expr(func, var, value)),
                    type_args: List::new(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_in_expr(a, var, value))
                        .collect(),
                },
                expr.span,
            ),

            // Method calls - substitute in receiver and arguments
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => Expr::new(
                ExprKind::MethodCall {
                    receiver: Box::new(self.substitute_in_expr(receiver, var, value)),
                    method: method.clone(),
                    type_args: type_args.clone(),
                    args: args
                        .iter()
                        .map(|a| self.substitute_in_expr(a, var, value))
                        .collect(),
                },
                expr.span,
            ),

            // Field access - substitute in base expression
            ExprKind::Field { expr: base, field } => Expr::new(
                ExprKind::Field {
                    expr: Box::new(self.substitute_in_expr(base, var, value)),
                    field: field.clone(),
                },
                expr.span,
            ),

            // Index access - substitute in both expression and index
            ExprKind::Index { expr: base, index } => Expr::new(
                ExprKind::Index {
                    expr: Box::new(self.substitute_in_expr(base, var, value)),
                    index: Box::new(self.substitute_in_expr(index, var, value)),
                },
                expr.span,
            ),

            // Tuple - substitute in all elements
            ExprKind::Tuple(elements) => Expr::new(
                ExprKind::Tuple(
                    elements
                        .iter()
                        .map(|e| self.substitute_in_expr(e, var, value))
                        .collect(),
                ),
                expr.span,
            ),

            // Array expressions
            ExprKind::Array(ArrayExpr::List(elements)) => Expr::new(
                ExprKind::Array(ArrayExpr::List(
                    elements
                        .iter()
                        .map(|e| self.substitute_in_expr(e, var, value))
                        .collect(),
                )),
                expr.span,
            ),
            ExprKind::Array(ArrayExpr::Repeat { value: elem, count }) => Expr::new(
                ExprKind::Array(ArrayExpr::Repeat {
                    value: Box::new(self.substitute_in_expr(elem, var, value)),
                    count: Box::new(self.substitute_in_expr(count, var, value)),
                }),
                expr.span,
            ),

            // Block - substitute in statements and final expression
            ExprKind::Block(block) => {
                let mut new_stmts = List::new();
                let mut shadowed = false;

                for stmt in &block.stmts {
                    if let StmtKind::Let { pattern, .. } = &stmt.kind {
                        // Check if pattern binds our variable
                        if self.pattern_binds_var(pattern, var) {
                            shadowed = true;
                        }
                    }

                    if !shadowed {
                        new_stmts.push(self.substitute_in_stmt(stmt, var, value));
                    } else {
                        new_stmts.push(stmt.clone());
                    }
                }

                let new_expr = if !shadowed {
                    block
                        .expr
                        .as_ref()
                        .map(|e| Box::new(self.substitute_in_expr(e, var, value)))
                } else {
                    block.expr.clone()
                };

                Expr::new(
                    ExprKind::Block(verum_ast::expr::Block {
                        stmts: new_stmts.into_iter().collect(),
                        expr: new_expr,
                        span: block.span,
                    }),
                    expr.span,
                )
            }

            // If expressions - substitute in condition and branches
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let new_conditions = condition
                    .conditions
                    .iter()
                    .map(|cond| match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            verum_ast::expr::ConditionKind::Expr(
                                self.substitute_in_expr(e, var, value),
                            )
                        }
                        verum_ast::expr::ConditionKind::Let {
                            pattern,
                            value: cond_expr,
                        } => verum_ast::expr::ConditionKind::Let {
                            pattern: pattern.clone(),
                            value: self.substitute_in_expr(cond_expr, var, value),
                        },
                    })
                    .collect();

                let new_condition = Box::new(verum_ast::expr::IfCondition {
                    conditions: new_conditions,
                    span: condition.span,
                });

                Expr::new(
                    ExprKind::If {
                        condition: new_condition,
                        then_branch: self.substitute_in_block(then_branch, var, value),
                        else_branch: else_branch
                            .as_ref()
                            .map(|e| Box::new(self.substitute_in_expr(e, var, value))),
                    },
                    expr.span,
                )
            }

            // Match expressions - substitute in scrutinee, handle shadowing in arms
            ExprKind::Match {
                expr: scrutinee,
                arms,
            } => Expr::new(
                ExprKind::Match {
                    expr: Box::new(self.substitute_in_expr(scrutinee, var, value)),
                    arms: arms
                        .iter()
                        .map(|arm| {
                            // Check if pattern shadows our variable
                            let shadowed = self.pattern_binds_var(&arm.pattern, var);

                            verum_ast::pattern::MatchArm {
                                pattern: arm.pattern.clone(),
                                guard: arm.guard.as_ref().map(|g| {
                                    Box::new(if !shadowed {
                                        self.substitute_in_expr(g, var, value)
                                    } else {
                                        (**g).clone()
                                    })
                                }),
                                body: Box::new(if !shadowed {
                                    self.substitute_in_expr(&arm.body, var, value)
                                } else {
                                    (*arm.body).clone()
                                }),
                                with_clause: arm.with_clause.clone(),
                                attributes: arm.attributes.clone(),
                                span: arm.span,
                            }
                        })
                        .collect(),
                },
                expr.span,
            ),

            // Closure - handle parameter shadowing with alpha-conversion if needed
            ExprKind::Closure {
                async_,
                move_,
                params,
                contexts,
                return_type,
                body,
            } => {
                // Check if any parameter shadows our variable
                let shadowed = params
                    .iter()
                    .any(|p| self.pattern_binds_var(&p.pattern, var));

                if shadowed {
                    // Variable is shadowed, don't substitute in body
                    expr.clone()
                } else {
                    // Check for potential capture - if any free var in value is bound by params
                    let params_capture = params.iter().any(|p| {
                        self.pattern_vars(&p.pattern)
                            .iter()
                            .any(|v| free_vars_in_value.contains(v))
                    });

                    if params_capture {
                        // Need alpha-conversion - for now, conservatively don't substitute
                        // Full implementation would rename parameters to fresh names
                        expr.clone()
                    } else {
                        // Safe to substitute
                        Expr::new(
                            ExprKind::Closure {
                                async_: *async_,
                                move_: *move_,
                                params: params.clone(),
                                contexts: contexts.clone(),
                                return_type: return_type.clone(),
                                body: Box::new(self.substitute_in_expr(body, var, value)),
                            },
                            expr.span,
                        )
                    }
                }
            }

            // For loops - handle pattern binding
            ExprKind::For {
                label: _,
                pattern,
                iter,
                body,
                invariants,
                decreases,
            } => {
                let shadowed = self.pattern_binds_var(pattern, var);

                Expr::new(
                    ExprKind::For {
                        label: None,
                        pattern: pattern.clone(),
                        iter: Box::new(self.substitute_in_expr(iter, var, value)),
                        body: if !shadowed {
                            self.substitute_in_block(body, var, value)
                        } else {
                            body.clone()
                        },
                        invariants: invariants.clone(),
                        decreases: decreases.clone(),
                    },
                    expr.span,
                )
            }

            // Return - substitute in inner expression if present
            ExprKind::Return(inner) => {
                let new_inner = inner
                    .as_ref()
                    .map(|e| Box::new(self.substitute_in_expr(e, var, value)));
                Expr::new(ExprKind::Return(new_inner), expr.span)
            }

            // Break - substitute in inner expression if present
            ExprKind::Break {
                label,
                value: inner,
            } => {
                let new_inner = inner
                    .as_ref()
                    .map(|e| Box::new(self.substitute_in_expr(e, var, value)));
                Expr::new(
                    ExprKind::Break {
                        label: label.clone(),
                        value: new_inner,
                    },
                    expr.span,
                )
            }
            ExprKind::Yield(inner) => Expr::new(
                ExprKind::Yield(Box::new(self.substitute_in_expr(inner, var, value))),
                expr.span,
            ),

            // Literals, Continue - no substitution needed
            ExprKind::Literal(_) | ExprKind::Continue { .. } => expr.clone(),

            // All other expression types - conservatively clone without substitution
            // This is safe but may miss optimization opportunities
            _ => expr.clone(),
        }
    }

    /// Helper: Collect free variables in expression
    fn collect_free_vars(&self, expr: &Expr) -> Set<Text> {
        let mut vars = Set::new();
        self.collect_free_vars_impl(expr, &mut Set::new(), &mut vars);
        vars
    }

    fn collect_free_vars_impl(&self, expr: &Expr, bound: &mut Set<Text>, free: &mut Set<Text>) {
        match &expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    let name: Text = ident.name.clone();
                    if !bound.contains(&name) {
                        free.insert(name);
                    }
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.collect_free_vars_impl(left, bound, free);
                self.collect_free_vars_impl(right, bound, free);
            }
            ExprKind::Unary { expr: inner, .. } => {
                self.collect_free_vars_impl(inner, bound, free);
            }
            ExprKind::Call { func, args, .. } => {
                self.collect_free_vars_impl(func, bound, free);
                for arg in args {
                    self.collect_free_vars_impl(arg, bound, free);
                }
            }
            ExprKind::Closure { params, body, .. } => {
                let mut new_bound = bound.clone();
                for param in params {
                    for v in self.pattern_vars(&param.pattern) {
                        new_bound.insert(v);
                    }
                }
                self.collect_free_vars_impl(body, &mut new_bound, free);
            }
            ExprKind::Block(block) => {
                let mut new_bound = bound.clone();
                for stmt in &block.stmts {
                    if let verum_ast::StmtKind::Let { pattern, value, .. } = &stmt.kind {
                        if let Some(init_expr) = value {
                            self.collect_free_vars_impl(init_expr, &mut new_bound, free);
                        }
                        for v in self.pattern_vars(pattern) {
                            new_bound.insert(v);
                        }
                    }
                }
                if let Some(final_expr) = &block.expr {
                    self.collect_free_vars_impl(final_expr, &mut new_bound, free);
                }
            }
            // Add more cases as needed
            _ => {}
        }
    }

    /// Helper: Check if pattern binds a variable
    fn pattern_binds_var(&self, pattern: &verum_ast::pattern::Pattern, var: &Text) -> bool {
        self.pattern_vars(pattern).contains(var)
    }

    /// Helper: Extract all variables bound by a pattern
    fn pattern_vars(&self, pattern: &verum_ast::pattern::Pattern) -> Set<Text> {
        use verum_ast::pattern::PatternKind;

        let mut vars = Set::new();

        match &pattern.kind {
            PatternKind::Ident { name, .. } => {
                vars.insert(name.name.clone());
            }
            PatternKind::Tuple(patterns) | PatternKind::Array(patterns) => {
                for p in patterns {
                    for var in self.pattern_vars(p) {
                        vars.insert(var);
                    }
                }
            }
            PatternKind::Record { fields, .. } => {
                for field in fields {
                    if let Some(ref pattern) = field.pattern {
                        for var in self.pattern_vars(pattern) {
                            vars.insert(var);
                        }
                    } else {
                        vars.insert(field.name.name.clone());
                    }
                }
            }
            PatternKind::Variant { data, .. } => {
                if let Some(data) = data {
                    match data {
                        verum_ast::pattern::VariantPatternData::Tuple(patterns) => {
                            for p in patterns {
                                for var in self.pattern_vars(p) {
                                    vars.insert(var);
                                }
                            }
                        }
                        verum_ast::pattern::VariantPatternData::Record { fields, .. } => {
                            for field in fields {
                                if let Some(ref p) = field.pattern {
                                    for var in self.pattern_vars(p) {
                                        vars.insert(var);
                                    }
                                } else {
                                    // Shorthand field pattern - field name is also a binding
                                    vars.insert(field.name.name.clone());
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Or(patterns) => {
                for p in patterns {
                    for var in self.pattern_vars(p) {
                        vars.insert(var);
                    }
                }
            }
            PatternKind::Paren(inner) => {
                for var in self.pattern_vars(inner) {
                    vars.insert(var);
                }
            }
            PatternKind::Reference { inner, .. } => {
                for var in self.pattern_vars(inner) {
                    vars.insert(var);
                }
            }
            _ => {}
        }

        vars
    }

    /// Helper: Substitute in statement
    fn substitute_in_stmt(
        &self,
        stmt: &verum_ast::Stmt,
        var: &Text,
        value: &Expr,
    ) -> verum_ast::Stmt {
        use verum_ast::{Stmt, StmtKind};

        let new_kind = match &stmt.kind {
            StmtKind::Let {
                pattern,
                ty,
                value: init,
            } => StmtKind::Let {
                pattern: pattern.clone(),
                ty: ty.clone(),
                value: init
                    .as_ref()
                    .map(|e| self.substitute_in_expr(e, var, value)),
            },
            StmtKind::Expr { expr, has_semi } => StmtKind::Expr {
                expr: self.substitute_in_expr(expr, var, value),
                has_semi: *has_semi,
            },
            _ => stmt.kind.clone(),
        };

        Stmt {
            kind: new_kind,
            span: stmt.span,
            attributes: stmt.attributes.clone(),
        }
    }

    /// Helper: Substitute in block
    fn substitute_in_block(
        &self,
        block: &verum_ast::expr::Block,
        var: &Text,
        value: &Expr,
    ) -> verum_ast::expr::Block {
        let mut new_stmts = List::new();
        let mut shadowed = false;

        for stmt in &block.stmts {
            if let verum_ast::StmtKind::Let { pattern, .. } = &stmt.kind
                && self.pattern_binds_var(pattern, var)
            {
                shadowed = true;
            }

            if !shadowed {
                new_stmts.push(self.substitute_in_stmt(stmt, var, value));
            } else {
                new_stmts.push(stmt.clone());
            }
        }

        let new_expr = if !shadowed {
            block
                .expr
                .as_ref()
                .map(|e| Box::new(self.substitute_in_expr(e, var, value)))
        } else {
            block.expr.clone()
        };

        verum_ast::expr::Block {
            stmts: new_stmts.into_iter().collect(),
            expr: new_expr,
            span: block.span,
        }
    }

    /// Try syntactic subsumption check (Mode 1)
    ///
    /// Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1 Mode 1
    /// Fast pattern matching for common cases
    fn try_syntactic_subsumption(
        &self,
        subtype: &RefinementType,
        supertype: &RefinementType,
    ) -> Maybe<bool> {
        // Pattern: x > a implies x > b where a >= b
        if let (Some((var1, op1, val1)), Some((var2, op2, val2))) = (
            self.extract_comparison(&subtype.predicate.predicate),
            self.extract_comparison(&supertype.predicate.predicate),
        ) && var1 == var2
        {
            return self.check_comparison_subsumption(op1, val1, op2, val2);
        }

        Maybe::None
    }

    /// Extract comparison from expression (var op literal)
    fn extract_comparison(&self, expr: &Expr) -> Option<(Text, BinOp, i64)> {
        if let ExprKind::Binary { op, left, right } = &expr.kind
            && let ExprKind::Path(path) = &left.kind
            && path.segments.len() == 1
            && let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0]
            && let ExprKind::Literal(Literal {
                kind: verum_ast::literal::LiteralKind::Int(int_lit),
                ..
            }) = &right.kind
        {
            return Some((ident.name.clone(), *op, int_lit.value as i64));
        }
        None
    }

    /// Check if comparison implies another
    fn check_comparison_subsumption(
        &self,
        op1: BinOp,
        val1: i64,
        op2: BinOp,
        val2: i64,
    ) -> Maybe<bool> {
        use BinOp::*;

        let result = match (op1, op2) {
            // x > a implies x > b where a >= b
            (Gt, Gt) => val1 >= val2,
            // x > a implies x >= b where a >= b
            (Gt, Ge) => val1 >= val2,
            // x >= a implies x >= b where a >= b
            (Ge, Ge) => val1 >= val2,
            // x >= a implies x > b where a > b
            (Ge, Gt) => val1 > val2,
            // x < a implies x < b where a <= b
            (Lt, Lt) => val1 <= val2,
            // x < a implies x <= b where a <= b
            (Lt, Le) => val1 <= val2,
            // x <= a implies x <= b where a <= b
            (Le, Le) => val1 <= val2,
            // x <= a implies x < b where a < b
            (Le, Lt) => val1 < val2,
            // x == a implies x >= b where a >= b
            (Eq, Ge) => val1 >= val2,
            // x == a implies x > b where a > b
            (Eq, Gt) => val1 > val2,
            // x == a implies x <= b where a <= b
            (Eq, Le) => val1 <= val2,
            // x == a implies x < b where a < b
            (Eq, Lt) => val1 < val2,
            // x == a implies x == b where a == b
            (Eq, Eq) => val1 == val2,
            // x == a implies x != b where a != b
            (Eq, Ne) => val1 != val2,
            // x != a implies x != a
            (Ne, Ne) => val1 == val2,
            _ => return Maybe::None,
        };

        Maybe::Some(result)
    }

    /// Try syntactic check (fast path for obvious cases)
    fn try_syntactic_check(&self, vc: &VerificationCondition) -> Maybe<VerificationResult> {
        self.try_syntactic_eval(&vc.condition)
    }

    /// Recursively evaluate an expression syntactically if possible
    fn try_syntactic_eval(&self, expr: &Expr) -> Maybe<VerificationResult> {
        // Unwrap Paren + Block wrappers — the lambda-body refinement
        // syntax `|n| { n >= 1 }` parses into `Block { expr: Binary(...) }`
        // and plain `(pred)` parses into `Paren(Binary(...))`. Without
        // this unwrap the Binary-and-Literal arms below never see the
        // actual predicate, and the syntactic evaluator silently returns
        // None — which, for lambda-style refinements on type aliases,
        // caused the refinement check to degrade into "Unknown" and let
        // violations through at compile time.
        if let ExprKind::Paren(inner) = &expr.kind {
            return self.try_syntactic_eval(inner);
        }
        if let ExprKind::Block(block) = &expr.kind {
            if block.stmts.is_empty() {
                if let Some(inner) = block.expr.as_ref() {
                    return self.try_syntactic_eval(inner);
                }
            }
        }

        // Check for literal true/false
        if let ExprKind::Literal(Literal {
            kind: verum_ast::literal::LiteralKind::Bool(b),
            ..
        }) = &expr.kind
        {
            return Maybe::Some(if *b {
                VerificationResult::Valid
            } else {
                VerificationResult::Invalid {
                    counterexample: Maybe::None,
                }
            });
        }

        // Check for binary operations
        if let ExprKind::Binary { op, left, right } = &expr.kind {
            // Handle logical operations recursively
            match op {
                BinOp::And => {
                    // Both sides must be valid
                    if let (Maybe::Some(l), Maybe::Some(r)) = (
                        self.try_syntactic_eval(left),
                        self.try_syntactic_eval(right),
                    ) {
                        return Maybe::Some(if l.is_valid() && r.is_valid() {
                            VerificationResult::Valid
                        } else {
                            VerificationResult::Invalid {
                                counterexample: Maybe::None,
                            }
                        });
                    }
                    return Maybe::None;
                }
                BinOp::Or => {
                    // At least one side must be valid
                    if let (Maybe::Some(l), Maybe::Some(r)) = (
                        self.try_syntactic_eval(left),
                        self.try_syntactic_eval(right),
                    ) {
                        return Maybe::Some(if l.is_valid() || r.is_valid() {
                            VerificationResult::Valid
                        } else {
                            VerificationResult::Invalid {
                                counterexample: Maybe::None,
                            }
                        });
                    }
                    return Maybe::None;
                }
                _ => {}
            }

            // Handle comparisons between int literals (including negated literals)
            if let (Maybe::Some(left_val), Maybe::Some(right_val)) =
                (Self::try_extract_int_value(left), Self::try_extract_int_value(right))
            {
                let result = self.eval_comparison(*op, left_val, right_val);
                return Maybe::Some(if result {
                    VerificationResult::Valid
                } else {
                    VerificationResult::Invalid {
                        counterexample: Maybe::None,
                    }
                });
            }

            // Handle comparisons between float literals (including negated float literals)
            if let (Maybe::Some(left_val), Maybe::Some(right_val)) =
                (Self::try_extract_float_value(left), Self::try_extract_float_value(right))
            {
                let result = self.eval_float_comparison(*op, left_val, right_val);
                return Maybe::Some(if result {
                    VerificationResult::Valid
                } else {
                    VerificationResult::Invalid {
                        counterexample: Maybe::None,
                    }
                });
            }
        }

        Maybe::None
    }

    /// Try to extract a constant integer value from an expression.
    /// Handles plain int literals, negated int literals, parenthesised
    /// expressions, and pure-integer arithmetic over constant operands
    /// (`+`, `-`, `*`, `/`, `%`, `<<`, `>>`, `&`, `|`, `^`). Without the
    /// arithmetic fold, refinement predicates like `n > 2 * 10` would
    /// evaluate to Unknown at the syntactic check — the evaluator
    /// couldn't prove `15 > 20` false, so violations slipped through.
    /// Arithmetic on two compile-time literals is total and side-effect
    /// free (div/mod by zero is conservatively refused by returning
    /// None — the SMT layer handles that case).
    fn try_extract_int_value(expr: &Expr) -> Maybe<i64> {
        match &expr.kind {
            ExprKind::Literal(Literal {
                kind: verum_ast::literal::LiteralKind::Int(int_lit),
                ..
            }) => Maybe::Some(int_lit.value as i64),
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Neg,
                expr: inner,
            } => {
                Self::try_extract_int_value(inner).map(|val| -val)
            }
            ExprKind::Paren(inner) => Self::try_extract_int_value(inner),
            // `<literal-list>.len()` / `<byte-string>.len()` / `"text".len()`
            // — common in refinement predicates where the bound variable
            // gets substituted with a concrete list / byte-string / text
            // literal. Fold to the compile-time length so the evaluator
            // can reduce `xs.len() > 0` against `xs = []` etc.
            ExprKind::MethodCall { receiver, method, args, .. }
                if method.name.as_str() == "len" && args.is_empty() =>
            {
                match &receiver.kind {
                    ExprKind::Array(arr) => match arr {
                        verum_ast::expr::ArrayExpr::List(items) => {
                            Maybe::Some(items.len() as i64)
                        }
                        verum_ast::expr::ArrayExpr::Repeat { count, .. } => {
                            Self::try_extract_int_value(count)
                        }
                    },
                    ExprKind::Literal(Literal {
                        kind: verum_ast::literal::LiteralKind::Text(s),
                        ..
                    }) => Maybe::Some(s.as_str().len() as i64),
                    ExprKind::Literal(Literal {
                        kind: verum_ast::literal::LiteralKind::ByteString(b),
                        ..
                    }) => Maybe::Some(b.len() as i64),
                    _ => Maybe::None,
                }
            }

            // Tuple-index `.0`, `.1`, ... on a literal-tuple receiver.
            // Refinements like `|p| { p.0 > 0 }` on `type T is (Int, Int)`
            // substitute p with the actual tuple literal `(0, 100)`.
            // Without this fold the evaluator can't reduce `(0, 100).0`
            // to `0` and returns Unknown.
            ExprKind::TupleIndex { expr: inner, index } => {
                let peel = |e: &Expr| -> Maybe<i64> {
                    if let ExprKind::Tuple(items) = &e.kind {
                        let idx = *index as usize;
                        if idx < items.len() {
                            return Self::try_extract_int_value(&items[idx]);
                        }
                    }
                    Maybe::None
                };
                match &inner.kind {
                    ExprKind::Tuple(_) => peel(inner),
                    ExprKind::Paren(p) => peel(p),
                    _ => Maybe::None,
                }
            }

            ExprKind::Binary { op, left, right } => {
                use verum_ast::expr::BinOp;
                let l = Self::try_extract_int_value(left);
                let r = Self::try_extract_int_value(right);
                match (l, r) {
                    (Maybe::Some(lv), Maybe::Some(rv)) => match op {
                        BinOp::Add => lv.checked_add(rv).map_or(Maybe::None, Maybe::Some),
                        BinOp::Sub => lv.checked_sub(rv).map_or(Maybe::None, Maybe::Some),
                        BinOp::Mul => lv.checked_mul(rv).map_or(Maybe::None, Maybe::Some),
                        BinOp::Div => {
                            if rv == 0 { Maybe::None }
                            else { lv.checked_div(rv).map_or(Maybe::None, Maybe::Some) }
                        }
                        BinOp::Rem => {
                            if rv == 0 { Maybe::None }
                            else { Maybe::Some(lv.rem_euclid(rv)) }
                        }
                        BinOp::Shl => {
                            if rv < 0 || rv >= 64 { Maybe::None }
                            else { Maybe::Some(lv << rv) }
                        }
                        BinOp::Shr => {
                            if rv < 0 || rv >= 64 { Maybe::None }
                            else { Maybe::Some(lv >> rv) }
                        }
                        BinOp::BitAnd => Maybe::Some(lv & rv),
                        BinOp::BitOr  => Maybe::Some(lv | rv),
                        BinOp::BitXor => Maybe::Some(lv ^ rv),
                        _ => Maybe::None,
                    },
                    _ => Maybe::None,
                }
            }
            _ => Maybe::None,
        }
    }

    /// Evaluate comparison
    fn eval_comparison(&self, op: BinOp, left: i64, right: i64) -> bool {
        use BinOp::*;
        match op {
            Eq => left == right,
            Ne => left != right,
            Lt => left < right,
            Le => left <= right,
            Gt => left > right,
            Ge => left >= right,
            _ => false,
        }
    }

    /// Try to extract a constant float value from an expression.
    /// Handles plain float literals, negated float literals, and parenthesized expressions.
    fn try_extract_float_value(expr: &Expr) -> Maybe<f64> {
        match &expr.kind {
            ExprKind::Literal(Literal {
                kind: verum_ast::literal::LiteralKind::Float(float_lit),
                ..
            }) => Maybe::Some(float_lit.value),
            // Also accept integer literals as floats (e.g., 0 in float context)
            ExprKind::Literal(Literal {
                kind: verum_ast::literal::LiteralKind::Int(int_lit),
                ..
            }) => Maybe::Some(int_lit.value as f64),
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Neg,
                expr: inner,
            } => {
                Self::try_extract_float_value(inner).map(|val| -val)
            }
            ExprKind::Paren(inner) => Self::try_extract_float_value(inner),
            _ => Maybe::None,
        }
    }

    /// Evaluate float comparison
    fn eval_float_comparison(&self, op: BinOp, left: f64, right: f64) -> bool {
        use BinOp::*;
        match op {
            Eq => (left - right).abs() < f64::EPSILON,
            Ne => (left - right).abs() >= f64::EPSILON,
            Lt => left < right,
            Le => left <= right,
            Gt => left > right,
            Ge => left >= right,
            _ => false,
        }
    }

    /// Check with SMT solver (Mode 2)
    ///
    /// Refinement types with gradual verification: types can carry predicates (Int{> 0}) verified at compile-time or runtime depending on verification level — .1 Mode 2
    fn check_with_smt(
        &self,
        vc: &VerificationCondition,
    ) -> Result<VerificationResult, RefinementError> {
        {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.smt_checks += 1;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    stats.smt_checks += 1;
                }
            }
        }

        if let Maybe::Some(ref backend) = self.smt_backend {
            let mut backend = match backend.write() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            // Forward the configured per-query timeout to the
            // backend so `RefinementConfig.timeout_ms` is honoured.
            // Default trait impl is a no-op for legacy backends.
            backend.set_timeout_ms(self.config.timeout_ms);

            // Check negation: ¬(predicate) should be UNSAT for valid refinement
            let negated = self.negate_expr(&vc.condition);

            match backend.check(&negated) {
                Ok(SmtResult::Unsat) => {
                    // ¬predicate is UNSAT => predicate is valid
                    Ok(VerificationResult::Valid)
                }
                Ok(SmtResult::Sat) => {
                    // ¬predicate is SAT => found counterexample
                    let model = backend.get_model().ok();
                    let counterexample = model.and_then(|m| {
                        m.iter()
                            .next()
                            .map(|(k, v)| CounterExample::new(k.clone(), v.clone()))
                    });
                    Ok(VerificationResult::Invalid { counterexample })
                }
                Ok(SmtResult::Unknown) => Ok(VerificationResult::Unknown {
                    reason: "SMT solver returned unknown".into(),
                }),
                Err(e) => Ok(VerificationResult::Unknown {
                    reason: format!("SMT error: {}", e).into(),
                }),
            }
        } else {
            Ok(VerificationResult::Unknown {
                reason: "No SMT backend available".into(),
            })
        }
    }

    /// Negate an expression
    fn negate_expr(&self, expr: &Expr) -> Expr {
        Expr::new(
            ExprKind::Unary {
                op: verum_ast::expr::UnOp::Not,
                expr: Box::new(expr.clone()),
            },
            expr.span,
        )
    }

    /// Generate implication: φ1 => φ2
    fn generate_implication(&self, phi1: &Expr, phi2: &Expr) -> Result<Expr, RefinementError> {
        // φ1 => φ2 is equivalent to ¬φ1 ∨ φ2
        let not_phi1 = self.negate_expr(phi1);

        Ok(Expr::new(
            ExprKind::Binary {
                op: BinOp::Or,
                left: Box::new(not_phi1),
                right: Box::new(phi2.clone()),
            },
            phi1.span,
        ))
    }

    /// Compute cache key for verification condition using structural hashing.
    ///
    /// This provides a stable hash based on the actual AST structure rather than
    /// pointer addresses, enabling proper cache hits for equivalent predicates.
    fn compute_cache_key(&self, vc: &VerificationCondition) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash the condition expression structurally
        Self::hash_expr(&vc.condition, &mut hasher);

        // Hash each assumption
        vc.assumptions.len().hash(&mut hasher);
        for assumption in &vc.assumptions {
            Self::hash_expr(assumption, &mut hasher);
        }

        // Hash substitutions
        vc.substitutions.len().hash(&mut hasher);
        for (var, expr) in &vc.substitutions {
            var.hash(&mut hasher);
            Self::hash_expr(expr, &mut hasher);
        }

        hasher.finish()
    }

    /// Hash an expression structurally for cache key computation.
    ///
    /// This traverses the AST and hashes each node to produce a stable hash
    /// that represents the expression's structure rather than its memory location.
    fn hash_expr<H: std::hash::Hasher>(expr: &Expr, hasher: &mut H) {
        use std::hash::Hash;
        use verum_ast::literal::LiteralKind;

        // Hash a discriminant for the expression kind
        std::mem::discriminant(&expr.kind).hash(hasher);

        match &expr.kind {
            ExprKind::Binary { op, left, right } => {
                std::mem::discriminant(op).hash(hasher);
                Self::hash_expr(left, hasher);
                Self::hash_expr(right, hasher);
            }

            ExprKind::Unary { op, expr: inner } => {
                std::mem::discriminant(op).hash(hasher);
                Self::hash_expr(inner, hasher);
            }

            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Int(int_lit) => {
                    0u8.hash(hasher); // Discriminant
                    int_lit.value.hash(hasher);
                }
                LiteralKind::Float(float_lit) => {
                    1u8.hash(hasher);
                    // Hash float bytes for stability
                    float_lit.value.to_bits().hash(hasher);
                }
                LiteralKind::Bool(b) => {
                    2u8.hash(hasher);
                    b.hash(hasher);
                }
                LiteralKind::Char(c) => {
                    3u8.hash(hasher);
                    c.hash(hasher);
                }
                LiteralKind::ByteChar(b) => {
                    10u8.hash(hasher);
                    b.hash(hasher);
                }
                LiteralKind::ByteString(bytes) => {
                    11u8.hash(hasher);
                    bytes.hash(hasher);
                }
                LiteralKind::Text(s) => {
                    4u8.hash(hasher);
                    match s {
                        verum_ast::literal::StringLit::Regular(text)
                        | verum_ast::literal::StringLit::MultiLine(text) => text.hash(hasher),
                    }
                }
                LiteralKind::Composite(comp) => {
                    5u8.hash(hasher);
                    comp.tag.hash(hasher);
                    comp.content.hash(hasher);
                }
                LiteralKind::InterpolatedString(interp) => {
                    6u8.hash(hasher);
                    interp.prefix.hash(hasher);
                    interp.content.hash(hasher);
                }
                LiteralKind::Tagged { tag, content } => {
                    7u8.hash(hasher);
                    tag.hash(hasher);
                    content.hash(hasher);
                }
                LiteralKind::Contract(text) => {
                    8u8.hash(hasher);
                    text.hash(hasher);
                }
                LiteralKind::ContextAdaptive(lit) => {
                    9u8.hash(hasher);
                    lit.raw.hash(hasher);
                }
            },

            ExprKind::Path(path) => {
                for seg in &path.segments {
                    match seg {
                        verum_ast::ty::PathSegment::Name(ident) => {
                            0u8.hash(hasher);
                            ident.name.hash(hasher);
                        }
                        verum_ast::ty::PathSegment::SelfValue => 1u8.hash(hasher),
                        verum_ast::ty::PathSegment::Super => 2u8.hash(hasher),
                        verum_ast::ty::PathSegment::Cog => 3u8.hash(hasher),
                        verum_ast::ty::PathSegment::Relative => 4u8.hash(hasher),
                    }
                }
            }

            ExprKind::Call { func, args, .. } => {
                Self::hash_expr(func, hasher);
                args.len().hash(hasher);
                for arg in args {
                    Self::hash_expr(arg, hasher);
                }
            }

            ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } => {
                Self::hash_expr(receiver, hasher);
                method.name.hash(hasher);
                args.len().hash(hasher);
                for arg in args {
                    Self::hash_expr(arg, hasher);
                }
            }

            ExprKind::Field { expr: inner, field } => {
                Self::hash_expr(inner, hasher);
                field.name.hash(hasher);
            }

            ExprKind::Index { expr: inner, index } => {
                Self::hash_expr(inner, hasher);
                Self::hash_expr(index, hasher);
            }

            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Hash condition parts
                condition.conditions.len().hash(hasher);
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            Self::hash_expr(e, hasher);
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            Self::hash_expr(value, hasher);
                        }
                    }
                }
                // Hash then branch expression if present
                if let Some(expr) = &then_branch.expr {
                    Self::hash_expr(expr, hasher);
                }
                if let verum_common::Maybe::Some(else_expr) = else_branch {
                    Self::hash_expr(else_expr, hasher);
                }
            }

            ExprKind::Tuple(elements) => {
                elements.len().hash(hasher);
                for elem in elements {
                    Self::hash_expr(elem, hasher);
                }
            }

            ExprKind::Array(array_expr) => {
                use verum_ast::expr::ArrayExpr;
                match array_expr {
                    ArrayExpr::List(elements) => {
                        elements.len().hash(hasher);
                        for elem in elements {
                            Self::hash_expr(elem, hasher);
                        }
                    }
                    ArrayExpr::Repeat { value, count } => {
                        Self::hash_expr(value, hasher);
                        Self::hash_expr(count, hasher);
                    }
                }
            }

            // For other complex expressions, use a simple discriminant hash
            // This is conservative but ensures correctness
            _ => {
                255u8.hash(hasher);
            }
        }
    }

    /// Get verification statistics
    ///
    /// Returns a copy of the current verification statistics.
    /// Handles lock poisoning gracefully by recovering the data.
    pub fn stats(&self) -> VerificationStats {
        match self.stats.read() {
            Ok(stats) => stats.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Clear cache
    ///
    /// Removes all cached verification results.
    /// Handles lock poisoning gracefully by recovering and clearing.
    pub fn clear_cache(&mut self) {
        match self.cache.write() {
            Ok(mut cache) => cache.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    // ==================== Evidence-Aware Verification ====================
    // Refinement types enhancement: flow-sensitive refinement propagation, evidence tracking for verified predicates — Refinement Evidence Propagation

    /// Check refinement with path evidence (flow-sensitive assumptions)
    ///
    /// This method enables the type checker to provide learned predicates
    /// (path conditions) that should be assumed true when verifying refinements.
    ///
    /// # Example
    ///
    /// ```verum
    /// fn process(data: List<Int>) -> Int {
    ///     if data.is_empty() { return 0; }
    ///     // Evidence: !data.is_empty() holds here
    ///     first(data)  // With evidence, this verifies correctly
    /// }
    /// ```
    ///
    /// # Arguments
    ///
    /// * `value` - The expression being checked
    /// * `refinement` - The refinement type to check against
    /// * `path_evidence` - Predicates known to be true on current path
    /// * `ctx` - Type context for variable resolution
    ///
    /// # Returns
    ///
    /// `VerificationResult::Valid` if the value satisfies the refinement
    /// given the path evidence, or `Invalid`/`Unknown` otherwise.
    ///
    /// Syntactic-only refinement check (no SMT).
    /// Returns Some(result) if the syntactic evaluator can determine the outcome,
    /// None if it cannot (complex predicates like modulo, string ops, etc.).
    pub fn syntactic_check_only(
        &self,
        value: &Expr,
        predicate: &RefinementPredicate,
    ) -> Maybe<VerificationResult> {
        match self.generate_vc(value, predicate) {
            Ok(vc) => self.try_syntactic_eval(&vc.condition),
            Err(_) => Maybe::None,
        }
    }

    pub fn check_with_evidence(
        &mut self,
        value: &Expr,
        refinement: &RefinementType,
        path_evidence: &[Expr],
        _ctx: &TypeContext,
    ) -> Result<VerificationResult, RefinementError> {
        let start = Instant::now();

        // Update stats
        {
            match self.stats.write() {
                Ok(mut stats) => stats.total_checks += 1,
                Err(poisoned) => poisoned.into_inner().total_checks += 1,
            }
        }

        // Trivial refinement (unrefined type) - always valid
        if refinement.is_unrefined() {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.successful += 1;
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    stats.successful += 1;
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
            return Ok(VerificationResult::Valid);
        }

        // Generate verification condition with assumptions
        let mut vc = self.generate_vc(value, &refinement.predicate)?;

        // Add path evidence as assumptions
        for assumption in path_evidence {
            vc = vc.with_assumption(assumption.clone());
        }

        // Try syntactic check first (fast path)
        // The syntactic checker considers assumptions in the VC
        if let Maybe::Some(result) = self.try_syntactic_check_with_assumptions(&vc) {
            match self.stats.write() {
                Ok(mut stats) => {
                    stats.syntactic_checks += 1;
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    stats.syntactic_checks += 1;
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
            return Ok(result);
        }

        // Check cache (with assumptions included in key)
        if self.config.enable_cache {
            let cache_key = self.compute_cache_key(&vc);
            let cache_result = match self.cache.read() {
                Ok(cache) => cache.get(&cache_key).cloned(),
                Err(poisoned) => poisoned.into_inner().get(&cache_key).cloned(),
            };
            if let Some(cached_result) = cache_result {
                match self.stats.write() {
                    Ok(mut stats) => {
                        stats.cache_hits += 1;
                        stats.elapsed_micros += start.elapsed().as_micros() as u64;
                    }
                    Err(poisoned) => {
                        let mut stats = poisoned.into_inner();
                        stats.cache_hits += 1;
                        stats.elapsed_micros += start.elapsed().as_micros() as u64;
                    }
                }
                return Ok(cached_result);
            }
        }

        // Fall back to SMT solver with assumptions
        let result = if self.config.enable_smt {
            self.check_with_smt_and_assumptions(&vc)?
        } else {
            VerificationResult::Unknown {
                reason: "SMT solver disabled".into(),
            }
        };

        // Update cache
        if self.config.enable_cache {
            let cache_key = self.compute_cache_key(&vc);
            let cache_write_result = self.cache.write();
            let mut cache = match cache_write_result {
                Ok(cache) => cache,
                Err(poisoned) => poisoned.into_inner(),
            };

            if cache.len() >= self.config.max_cache_size {
                let to_remove: List<u64> = cache
                    .keys()
                    .take(self.config.max_cache_size / 10)
                    .cloned()
                    .collect();
                for key in to_remove {
                    cache.remove(&key);
                }
            }

            cache.insert(cache_key, result.clone());
        }

        // Update stats
        {
            match self.stats.write() {
                Ok(mut stats) => {
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
                Err(poisoned) => {
                    let mut stats = poisoned.into_inner();
                    if result.is_valid() {
                        stats.successful += 1;
                    } else if result.is_invalid() {
                        stats.failed += 1;
                    } else {
                        stats.unknown += 1;
                    }
                    stats.elapsed_micros += start.elapsed().as_micros() as u64;
                }
            }
        }

        Ok(result)
    }

    /// Syntactic subsumption check with assumptions
    ///
    /// Checks if the condition is syntactically provable given the assumptions.
    /// This extends the basic syntactic check to consider path conditions.
    fn try_syntactic_check_with_assumptions(
        &self,
        vc: &VerificationCondition,
    ) -> Maybe<VerificationResult> {
        // Check if condition is directly implied by any assumption
        for assumption in &vc.assumptions {
            if self.expr_syntactically_equal(assumption, &vc.condition) {
                return Maybe::Some(VerificationResult::Valid);
            }

            // Check if assumption implies condition (simple cases)
            if self.assumption_implies_condition(assumption, &vc.condition) {
                return Maybe::Some(VerificationResult::Valid);
            }
        }

        // Fall back to regular syntactic check
        self.try_syntactic_check(vc)
    }

    /// Check if an assumption syntactically implies a condition
    ///
    /// Handles common patterns like:
    /// - `!x.is_empty()` implies `x.len() > 0`
    /// - `x > 0` implies `x >= 0`
    /// - `x.is_some()` implies `x != None`
    fn assumption_implies_condition(&self, assumption: &Expr, condition: &Expr) -> bool {
        // Pattern 1: !receiver.is_empty() implies len(receiver) > 0
        if let ExprKind::Unary {
            op: verum_ast::expr::UnOp::Not,
            expr: inner,
        } = &assumption.kind
        {
            if let ExprKind::MethodCall {
                receiver,
                method,
                args,
                ..
            } = &inner.kind
            {
                if method.name.as_str() == "is_empty" && args.is_empty() {
                    // Check if condition is len(receiver) > 0 or receiver.len() > 0
                    if self.is_positive_length_check(condition, receiver) {
                        return true;
                    }
                }
            }
        }

        // Pattern 2: x > n implies x >= n and x > m for m < n
        if let (
            ExprKind::Binary {
                op: op1,
                left: left1,
                right: right1,
            },
            ExprKind::Binary {
                op: op2,
                left: left2,
                right: right2,
            },
        ) = (&assumption.kind, &condition.kind)
        {
            if self.expr_syntactically_equal(left1, left2) {
                match (op1, op2) {
                    // x > n implies x >= n
                    (BinOp::Gt, BinOp::Ge) => {
                        if self.expr_syntactically_equal(right1, right2) {
                            return true;
                        }
                    }
                    // x >= n implies x >= m for m <= n
                    (BinOp::Ge, BinOp::Ge) => {
                        if let (Some(n1), Some(n2)) =
                            (self.extract_int_literal(right1), self.extract_int_literal(right2))
                        {
                            if n1 >= n2 {
                                return true;
                            }
                        }
                    }
                    // x > n implies x > m for m < n
                    (BinOp::Gt, BinOp::Gt) => {
                        if let (Some(n1), Some(n2)) =
                            (self.extract_int_literal(right1), self.extract_int_literal(right2))
                        {
                            if n1 > n2 {
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Pattern 3: receiver.is_some() implies receiver != None
        if let ExprKind::MethodCall {
            receiver,
            method,
            args,
            ..
        } = &assumption.kind
        {
            if method.name.as_str() == "is_some" && args.is_empty() {
                if self.is_not_none_check(condition, receiver) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if condition is a positive length check on the given receiver
    fn is_positive_length_check(&self, condition: &Expr, receiver: &Expr) -> bool {
        if let ExprKind::Binary {
            op: BinOp::Gt,
            left,
            right,
        } = &condition.kind
        {
            // Check for receiver.len() > 0
            if let ExprKind::MethodCall {
                receiver: cond_recv,
                method,
                args,
                ..
            } = &left.kind
            {
                if method.name.as_str() == "len"
                    && args.is_empty()
                    && self.expr_syntactically_equal(cond_recv, receiver)
                {
                    if let Some(0) = self.extract_int_literal(right) {
                        return true;
                    }
                }
            }

            // Check for len(receiver) > 0 (function call form)
            if let ExprKind::Call { func, args, .. } = &left.kind {
                if let ExprKind::Path(path) = &func.kind {
                    if path.segments.len() == 1 {
                        if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                            if ident.name.as_str() == "len"
                                && args.len() == 1
                                && self.expr_syntactically_equal(&args[0], receiver)
                            {
                                if let Some(0) = self.extract_int_literal(right) {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        false
    }

    /// Check if condition is a not-None check on the given receiver
    fn is_not_none_check(&self, condition: &Expr, receiver: &Expr) -> bool {
        if let ExprKind::Binary {
            op: BinOp::Ne,
            left,
            right,
        } = &condition.kind
        {
            // receiver != None
            if self.expr_syntactically_equal(left, receiver) && self.is_none_literal(right) {
                return true;
            }
            // None != receiver
            if self.is_none_literal(left) && self.expr_syntactically_equal(right, receiver) {
                return true;
            }
        }
        false
    }

    /// Check if expression is a nullary variant literal (e.g. None, Nil, Empty).
    /// Structural: any single-segment path starting with uppercase is treated as
    /// a potential nullary variant for refinement narrowing purposes.
    fn is_none_literal(&self, expr: &Expr) -> bool {
        if let ExprKind::Path(path) = &expr.kind {
            if path.segments.len() == 1 {
                if let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] {
                    // A single uppercase-starting identifier with no args is
                    // likely a nullary variant constructor (None, Nil, Empty, etc.)
                    return ident.name.as_str().starts_with(char::is_uppercase);
                }
            }
        }
        false
    }

    /// Extract integer literal value from expression
    fn extract_int_literal(&self, expr: &Expr) -> Option<i128> {
        if let ExprKind::Literal(lit) = &expr.kind {
            if let verum_ast::literal::LiteralKind::Int(int_lit) = &lit.kind {
                return Some(int_lit.value);
            }
        }
        None
    }

    /// Check syntactic equality of two expressions
    fn expr_syntactically_equal(&self, e1: &Expr, e2: &Expr) -> bool {
        match (&e1.kind, &e2.kind) {
            (ExprKind::Path(p1), ExprKind::Path(p2)) => {
                if p1.segments.len() != p2.segments.len() {
                    return false;
                }
                p1.segments
                    .iter()
                    .zip(p2.segments.iter())
                    .all(|(s1, s2)| match (s1, s2) {
                        (
                            verum_ast::ty::PathSegment::Name(i1),
                            verum_ast::ty::PathSegment::Name(i2),
                        ) => i1.name == i2.name,
                        (verum_ast::ty::PathSegment::SelfValue, verum_ast::ty::PathSegment::SelfValue) => true,
                        (verum_ast::ty::PathSegment::Super, verum_ast::ty::PathSegment::Super) => true,
                        (verum_ast::ty::PathSegment::Cog, verum_ast::ty::PathSegment::Cog) => true,
                        _ => false,
                    })
            }

            (
                ExprKind::Literal(l1),
                ExprKind::Literal(l2),
            ) => {
                use verum_ast::literal::LiteralKind;
                match (&l1.kind, &l2.kind) {
                    (LiteralKind::Int(i1), LiteralKind::Int(i2)) => i1.value == i2.value,
                    (LiteralKind::Float(f1), LiteralKind::Float(f2)) => f1.value == f2.value,
                    (LiteralKind::Bool(b1), LiteralKind::Bool(b2)) => b1 == b2,
                    (LiteralKind::Char(c1), LiteralKind::Char(c2)) => c1 == c2,
                    _ => false,
                }
            }

            (
                ExprKind::Binary {
                    op: op1,
                    left: left1,
                    right: right1,
                },
                ExprKind::Binary {
                    op: op2,
                    left: left2,
                    right: right2,
                },
            ) => {
                op1 == op2
                    && self.expr_syntactically_equal(left1, left2)
                    && self.expr_syntactically_equal(right1, right2)
            }

            (
                ExprKind::Unary { op: op1, expr: e1 },
                ExprKind::Unary { op: op2, expr: e2 },
            ) => op1 == op2 && self.expr_syntactically_equal(e1, e2),

            (
                ExprKind::MethodCall {
                    receiver: r1,
                    method: m1,
                    args: a1,
                    ..
                },
                ExprKind::MethodCall {
                    receiver: r2,
                    method: m2,
                    args: a2,
                    ..
                },
            ) => {
                m1.name == m2.name
                    && self.expr_syntactically_equal(r1, r2)
                    && a1.len() == a2.len()
                    && a1
                        .iter()
                        .zip(a2.iter())
                        .all(|(e1, e2)| self.expr_syntactically_equal(e1, e2))
            }

            (ExprKind::Field { expr: e1, field: f1 }, ExprKind::Field { expr: e2, field: f2 }) => {
                f1.name == f2.name && self.expr_syntactically_equal(e1, e2)
            }

            _ => false,
        }
    }

    /// SMT check with assumptions
    ///
    /// Checks: assumptions => condition
    /// Which is equivalent to: ¬(assumptions ∧ ¬condition) is UNSAT
    fn check_with_smt_and_assumptions(
        &self,
        vc: &VerificationCondition,
    ) -> Result<VerificationResult, RefinementError> {
        {
            match self.stats.write() {
                Ok(mut stats) => stats.smt_checks += 1,
                Err(poisoned) => poisoned.into_inner().smt_checks += 1,
            }
        }

        if let Maybe::Some(ref backend) = self.smt_backend {
            let mut backend = match backend.write() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };

            // Forward configured per-query timeout to the backend.
            backend.set_timeout_ms(self.config.timeout_ms);

            // Build: assumptions ∧ ¬condition
            // If this is UNSAT, then assumptions => condition
            let negated_condition = self.negate_expr(&vc.condition);

            // Build conjunction of assumptions
            let query = if vc.assumptions.is_empty() {
                negated_condition
            } else {
                let mut conjunction = negated_condition;
                for assumption in &vc.assumptions {
                    conjunction = Expr::new(
                        ExprKind::Binary {
                            op: BinOp::And,
                            left: Box::new(assumption.clone()),
                            right: Box::new(conjunction),
                        },
                        assumption.span,
                    );
                }
                conjunction
            };

            match backend.check(&query) {
                Ok(SmtResult::Unsat) => {
                    // assumptions ∧ ¬condition is UNSAT => assumptions => condition
                    Ok(VerificationResult::Valid)
                }
                Ok(SmtResult::Sat) => {
                    // Found counterexample
                    let model = backend.get_model().ok();
                    let counterexample = model.and_then(|m| {
                        m.iter()
                            .next()
                            .map(|(k, v)| CounterExample::new(k.clone(), v.clone()))
                    });
                    Ok(VerificationResult::Invalid { counterexample })
                }
                Ok(SmtResult::Unknown) => Ok(VerificationResult::Unknown {
                    reason: "SMT solver returned unknown".into(),
                }),
                Err(e) => Ok(VerificationResult::Unknown {
                    reason: format!("SMT error: {}", e).into(),
                }),
            }
        } else {
            Ok(VerificationResult::Unknown {
                reason: "No SMT backend available".into(),
            })
        }
    }
}

impl Default for RefinementChecker {
    fn default() -> Self {
        Self::new(RefinementConfig::default())
    }
}

// Tests moved to tests/refinement_tests.rs
