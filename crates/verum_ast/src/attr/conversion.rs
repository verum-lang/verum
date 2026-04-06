//! Attribute conversion implementations for typed attributes.
//!
//! This module provides:
//! - Helper functions for extracting values from `Expr` nodes
//! - `FromAttribute` implementations for all typed attributes in `typed.rs`
//!
//! # Examples
//!
//! ```rust
//! use verum_ast::attr::{Attribute, InlineAttr, FromAttribute};
//!
//! fn process_inline(attr: &Attribute) {
//!     if let Ok(inline) = InlineAttr::from_attribute(attr) {
//!         println!("Inline mode: {:?}", inline.mode);
//!     }
//! }
//! ```

use crate::expr::{Expr, ExprKind};
use crate::literal::{IntLit, LiteralKind, StringLit};
use crate::span::Span;
use verum_common::{List, Maybe, Text};

use super::typed::{
    AccessPattern, AccessPatternAttr, AlignAttr, AliasAttr, AssumeAttr, BlackBoxAttr,
    ColdAttr, ConstEvalAttr, ConstEvalMode, CpuDispatchAttr, DeadlockDetectionAttr,
    DifferentiableAttr, EnsuresAttr, ExportAttr, FeatureAttr, GhostAttr, HotAttr,
    InitPriorityAttr, InlineAttr, InlineMode, InvariantAttr, IvdepAttr, LikelihoodAttr,
    LinkageAttr, LinkageKind, LinkNameAttr, LockLevelAttr, LtoAttr, LtoMode, MultiversionAttr,
    LlvmOnlyAttr, MultiversionVariant, NakedAttr, NoAliasAttr, NoMangleAttr, NoReturnAttr, OptimizationLevel,
    OptimizeAttr, OptimizeBarrierAttr, ParallelAttr, PerformanceContract, PgoAttr, PrefetchAccess,
    PrefetchAttr, Profile, ProfileAttr, ReduceAttr, ReductionOp, Repr, ReprAttr, RequiresAttr,
    SectionAttr, SpecializeAttr, StdAttr, SymbolVisibility, TaggedLiteralAttr, TargetCpuAttr,
    TargetFeatureAttr, UnrollAttr, UnrollMode, UsedAttr, VectorizeAttr, VectorizeMode,
    VerificationMode, VerifyAttr, VisibilityAttr, WeakAttr,
};
use super::{Attribute, AttributeConversionError, FromAttribute};

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Extract an identifier from an expression.
///
/// Works with:
/// - `ExprKind::Path` with a single segment
/// - Identifier expressions
///
/// # Examples
///
/// ```rust
/// use verum_ast::attr::conversion::extract_ident;
/// use verum_ast::expr::{Expr, ExprKind};
/// use verum_ast::ty::{Path, Ident};
/// use verum_ast::span::Span;
///
/// let expr = Expr::ident(Ident::new("always", Span::default()));
/// let ident = extract_ident(&expr).unwrap();
/// assert_eq!(ident.as_str(), "always");
/// ```
pub fn extract_ident(expr: &Expr) -> Result<Text, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                Ok(ident.name.clone())
            } else {
                Err(AttributeConversionError::invalid_args(
                    format!("expected identifier, got path '{}'", path),
                    expr.span,
                ))
            }
        }
        _ => Err(AttributeConversionError::invalid_args(
            "expected identifier",
            expr.span,
        )),
    }
}

/// Extract a string literal from an expression.
///
/// Works with:
/// - `ExprKind::Literal(LiteralKind::Text(...))`
///
/// # Examples
///
/// ```rust
/// use verum_ast::attr::conversion::extract_string;
/// use verum_ast::expr::Expr;
/// use verum_ast::literal::Literal;
/// use verum_ast::span::Span;
/// use verum_common::Text;
///
/// let expr = Expr::literal(Literal::string(Text::from("hello"), Span::default()));
/// let s = extract_string(&expr).unwrap();
/// assert_eq!(s.as_str(), "hello");
/// ```
pub fn extract_string(expr: &Expr) -> Result<Text, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Text(s) => Ok(s.into_string_ref()),
            _ => Err(AttributeConversionError::invalid_args(
                "expected string literal",
                expr.span,
            )),
        },
        _ => Err(AttributeConversionError::invalid_args(
            "expected string literal",
            expr.span,
        )),
    }
}

/// Extension trait to get string reference from StringLit
trait StringLitExt {
    fn into_string_ref(&self) -> Text;
}

impl StringLitExt for StringLit {
    fn into_string_ref(&self) -> Text {
        Text::from(self.as_str())
    }
}

/// Extract an integer literal from an expression.
///
/// Works with:
/// - `ExprKind::Literal(LiteralKind::Int(...))`
///
/// Returns the value as i128 (can be downcast to smaller types if needed).
pub fn extract_int(expr: &Expr) -> Result<i128, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Int(IntLit { value, .. }) => Ok(*value),
            _ => Err(AttributeConversionError::invalid_args(
                "expected integer literal",
                expr.span,
            )),
        },
        _ => Err(AttributeConversionError::invalid_args(
            "expected integer literal",
            expr.span,
        )),
    }
}

/// Extract a positive integer (u32) from an expression.
///
/// Returns an error if the value is negative or too large for u32.
pub fn extract_u32(expr: &Expr) -> Result<u32, AttributeConversionError> {
    let value = extract_int(expr)?;
    if value < 0 {
        return Err(AttributeConversionError::invalid_args(
            "expected positive integer",
            expr.span,
        ));
    }
    u32::try_from(value)
        .map_err(|_| AttributeConversionError::invalid_args("integer too large for u32", expr.span))
}

/// Extract a positive integer (u64) from an expression.
///
/// Returns an error if the value is negative or too large for u64.
pub fn extract_u64(expr: &Expr) -> Result<u64, AttributeConversionError> {
    let value = extract_int(expr)?;
    if value < 0 {
        return Err(AttributeConversionError::invalid_args(
            "expected positive integer",
            expr.span,
        ));
    }
    u64::try_from(value)
        .map_err(|_| AttributeConversionError::invalid_args("integer too large for u64", expr.span))
}

/// Extract a boolean literal from an expression.
///
/// Works with:
/// - `ExprKind::Literal(LiteralKind::Bool(...))`
/// - Identifiers "true" and "false"
pub fn extract_bool(expr: &Expr) -> Result<bool, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(b) => Ok(*b),
            _ => Err(AttributeConversionError::invalid_args(
                "expected boolean literal",
                expr.span,
            )),
        },
        ExprKind::Path(path) => {
            if let Some(ident) = path.as_ident() {
                match ident.name.as_str() {
                    "true" => Ok(true),
                    "false" => Ok(false),
                    _ => Err(AttributeConversionError::invalid_args(
                        "expected boolean literal (true/false)",
                        expr.span,
                    )),
                }
            } else {
                Err(AttributeConversionError::invalid_args(
                    "expected boolean literal",
                    expr.span,
                ))
            }
        }
        _ => Err(AttributeConversionError::invalid_args(
            "expected boolean literal",
            expr.span,
        )),
    }
}

/// Extract a floating-point literal from an expression.
///
/// Works with:
/// - `ExprKind::Literal(LiteralKind::Float(...))`
/// - Integer literals (converted to f64)
pub fn extract_float(expr: &Expr) -> Result<f64, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Float(f) => Ok(f.value),
            LiteralKind::Int(i) => Ok(i.value as f64),
            _ => Err(AttributeConversionError::invalid_args(
                "expected floating-point literal",
                expr.span,
            )),
        },
        _ => Err(AttributeConversionError::invalid_args(
            "expected floating-point literal",
            expr.span,
        )),
    }
}

/// Extract a named argument from a list of expressions.
///
/// Looks for expressions of the form `name = value` or `name: value`.
pub fn extract_named_arg<'a>(args: &'a [Expr], name: &str) -> Option<&'a Expr> {
    for arg in args {
        // Check for NamedArg { name, value } (colon syntax: name: value)
        if let ExprKind::NamedArg { name: arg_name, value } = &arg.kind {
            if arg_name.as_str() == name {
                return Some(value);
            }
        }
        // Also check for Binary Assign (equals syntax: name = value)
        if let ExprKind::Binary { op, left, right } = &arg.kind {
            use crate::expr::BinOp;
            if *op == BinOp::Assign {
                if let Ok(arg_name) = extract_ident(left) {
                    if arg_name.as_str() == name {
                        return Some(right);
                    }
                }
            }
        }
    }
    None
}

/// Extract a list of identifiers from expression arguments.
///
/// Used for attributes like `@profile(application, systems)`.
pub fn extract_ident_list(args: &[Expr]) -> Result<List<Text>, AttributeConversionError> {
    let mut result = List::new();
    for arg in args {
        result.push(extract_ident(arg)?);
    }
    Ok(result)
}

/// Extract a list of strings from expression arguments or a single array expression.
///
/// Used for attributes like `@feature(enable: ["unsafe", "inline_asm"])`.
pub fn extract_string_list(expr: &Expr) -> Result<List<Text>, AttributeConversionError> {
    match &expr.kind {
        ExprKind::Array(crate::expr::ArrayExpr::List(exprs)) => {
            let mut result = List::new();
            for e in exprs.iter() {
                result.push(extract_string(e)?);
            }
            Ok(result)
        }
        _ => {
            // Single string
            let s = extract_string(expr)?;
            Ok(vec![s].into())
        }
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - CORE ATTRIBUTES
// =============================================================================

impl FromAttribute for ProfileAttr {
    const NAME: &'static str = "profile";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let profiles = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@profile requires at least one profile argument (application, systems, research)",
                    attr.span,
                ));
            }
            Maybe::Some(args) => {
                let mut profiles = List::new();
                for arg in args.iter() {
                    let name = extract_ident(arg)?;
                    match Profile::from_str(name.as_str()) {
                        Maybe::Some(p) => profiles.push(p),
                        Maybe::None => {
                            return Err(AttributeConversionError::invalid_args(
                                format!(
                                    "unknown profile '{}', expected: application, systems, research",
                                    name
                                ),
                                arg.span,
                            ));
                        }
                    }
                }
                profiles
            }
        };

        Ok(ProfileAttr::new(profiles, attr.span))
    }
}

impl FromAttribute for FeatureAttr {
    const NAME: &'static str = "feature";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let features = match &attr.args {
            Maybe::None => List::new(),
            Maybe::Some(args) => {
                // Look for enable: [...] argument
                if let Some(enable_expr) = extract_named_arg(args, "enable") {
                    extract_string_list(enable_expr)?
                } else if !args.is_empty() {
                    // Fallback: treat all args as feature names
                    extract_ident_list(args)?
                } else {
                    List::new()
                }
            }
        };

        Ok(FeatureAttr::new(features, attr.span))
    }
}

impl FromAttribute for StdAttr {
    const NAME: &'static str = "std";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let context_group = match &attr.args {
            Maybe::None => Maybe::None,
            Maybe::Some(args) if args.is_empty() => Maybe::None,
            Maybe::Some(args) if args.len() == 1 => Maybe::Some(extract_ident(&args[0])?),
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@std takes at most one context group argument",
                    attr.span,
                ));
            }
        };

        Ok(StdAttr::new(context_group, attr.span))
    }
}

impl FromAttribute for SpecializeAttr {
    const NAME: &'static str = "specialize";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut negative = false;
        let mut rank = Maybe::None;
        let when_clause = Maybe::None; // WhereClause parsing is complex, skip for now

        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                // Check for "negative"
                if let Ok(name) = extract_ident(arg) {
                    if name.as_str() == "negative" {
                        negative = true;
                        continue;
                    }
                }

                // Check for rank = N
                if let Some(rank_expr) = extract_named_arg(std::slice::from_ref(arg), "rank") {
                    let val = extract_int(rank_expr)?;
                    rank = Maybe::Some(val as i32);
                }
            }
        }

        Ok(SpecializeAttr::new(negative, rank, when_clause, attr.span))
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - CONCURRENCY ATTRIBUTES
// =============================================================================

impl FromAttribute for LockLevelAttr {
    const NAME: &'static str = "lock_level";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let level = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@lock_level requires a level argument: @lock_level(level: N)",
                    attr.span,
                ));
            }
            Maybe::Some(args) => {
                // Look for level: N or just N
                if let Some(level_expr) = extract_named_arg(args, "level") {
                    extract_u32(level_expr)?
                } else if args.len() == 1 {
                    extract_u32(&args[0])?
                } else {
                    return Err(AttributeConversionError::invalid_args(
                        "@lock_level requires a level argument",
                        attr.span,
                    ));
                }
            }
        };

        Ok(LockLevelAttr::new(level, attr.span))
    }
}

impl FromAttribute for DeadlockDetectionAttr {
    const NAME: &'static str = "deadlock_detection";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut enabled = true;
        let mut timeout_ms = Maybe::None;

        if let Maybe::Some(args) = &attr.args {
            if let Some(enabled_expr) = extract_named_arg(args, "enabled") {
                enabled = extract_bool(enabled_expr)?;
            }
            if let Some(timeout_expr) = extract_named_arg(args, "timeout") {
                timeout_ms = Maybe::Some(extract_u64(timeout_expr)?);
            }
        }

        Ok(DeadlockDetectionAttr::new(enabled, timeout_ms, attr.span))
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - OPTIMIZATION ATTRIBUTES
// =============================================================================

impl FromAttribute for InlineAttr {
    const NAME: &'static str = "inline";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mode = match &attr.args {
            Maybe::None => InlineMode::Suggest,
            Maybe::Some(args) if args.is_empty() => InlineMode::Suggest,
            Maybe::Some(args) if args.len() == 1 => {
                let mode_name = extract_ident(&args[0])?;
                match InlineMode::from_str(mode_name.as_str()) {
                    Maybe::Some(m) => m,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown inline mode '{}', expected: always, never, release",
                                mode_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@inline takes at most one mode argument",
                    attr.span,
                ));
            }
        };

        Ok(InlineAttr::new(mode, attr.span))
    }
}

impl FromAttribute for ColdAttr {
    const NAME: &'static str = "cold";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        // @cold takes no arguments
        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@cold takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ColdAttr::new(attr.span))
    }
}

impl FromAttribute for HotAttr {
    const NAME: &'static str = "hot";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        // @hot takes no arguments
        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@hot takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(HotAttr::new(attr.span))
    }
}

impl FromAttribute for OptimizeAttr {
    const NAME: &'static str = "optimize";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let level = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@optimize requires a level: none, size, speed, balanced",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let level_name = extract_ident(&args[0])?;
                match OptimizationLevel::from_str(level_name.as_str()) {
                    Maybe::Some(l) => l,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown optimization level '{}', expected: none, size, speed, balanced",
                                level_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@optimize takes exactly one level argument",
                    attr.span,
                ));
            }
        };

        Ok(OptimizeAttr::new(level, attr.span))
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - VECTORIZATION & LOOP ATTRIBUTES
// =============================================================================

impl FromAttribute for VectorizeAttr {
    const NAME: &'static str = "vectorize";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut mode = VectorizeMode::Auto;
        let mut width = Maybe::None;

        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                // Check for mode identifier
                if let Ok(name) = extract_ident(arg) {
                    if let Maybe::Some(m) = VectorizeMode::from_str(name.as_str()) {
                        mode = m;
                        continue;
                    }
                }

                // Check for width: N
                if let Some(width_expr) = extract_named_arg(std::slice::from_ref(arg), "width") {
                    width = Maybe::Some(extract_u32(width_expr)?);
                }

                // Check for integer (width without keyword)
                if let Ok(n) = extract_u32(arg) {
                    width = Maybe::Some(n);
                }
            }
        }

        let mut result = VectorizeAttr::new(mode, attr.span);
        result.width = width;
        Ok(result)
    }
}

impl FromAttribute for UnrollAttr {
    const NAME: &'static str = "unroll";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mode = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@unroll requires either a count (N), 'full', or use @no_unroll",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                // Check for "full"
                if let Ok(name) = extract_ident(&args[0]) {
                    if name.as_str() == "full" {
                        UnrollMode::Full
                    } else {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown unroll mode '{}', expected: full or integer count",
                                name
                            ),
                            args[0].span,
                        ));
                    }
                } else {
                    // Try integer
                    let count = extract_u32(&args[0])?;
                    UnrollMode::Count(count)
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@unroll takes exactly one argument",
                    attr.span,
                ));
            }
        };

        Ok(UnrollAttr::new(mode, attr.span))
    }
}

/// Handles @no_unroll attribute
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoUnrollAttr;

impl FromAttribute for NoUnrollAttr {
    const NAME: &'static str = "no_unroll";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }
        Ok(NoUnrollAttr)
    }
}

impl FromAttribute for ParallelAttr {
    const NAME: &'static str = "parallel";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        // @parallel takes no arguments
        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@parallel takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ParallelAttr::new(attr.span))
    }
}

impl FromAttribute for ReduceAttr {
    const NAME: &'static str = "reduce";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let op = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@reduce requires an operation: +, *, min, max, &, |, ^, &&, ||",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let op_name = extract_ident(&args[0])?;
                match ReductionOp::from_str(op_name.as_str()) {
                    Maybe::Some(op) => op,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown reduction operation '{}', expected: +, *, min, max, &, |, ^, &&, ||",
                                op_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@reduce takes exactly one operation argument",
                    attr.span,
                ));
            }
        };

        Ok(ReduceAttr::new(op, attr.span))
    }
}

impl FromAttribute for NoAliasAttr {
    const NAME: &'static str = "no_alias";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@no_alias takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NoAliasAttr::new(attr.span))
    }
}

impl FromAttribute for IvdepAttr {
    const NAME: &'static str = "ivdep";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@ivdep takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(IvdepAttr::new(attr.span))
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - MEMORY/LAYOUT ATTRIBUTES
// =============================================================================

impl FromAttribute for AlignAttr {
    const NAME: &'static str = "align";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let alignment = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@align requires an alignment value (power of 2)",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let n = extract_u32(&args[0])?;
                if !n.is_power_of_two() {
                    return Err(AttributeConversionError::invalid_args(
                        format!("alignment must be a power of 2, got {}", n),
                        args[0].span,
                    ));
                }
                n
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@align takes exactly one alignment argument",
                    attr.span,
                ));
            }
        };

        Ok(AlignAttr::new(alignment, attr.span))
    }
}

impl FromAttribute for ReprAttr {
    const NAME: &'static str = "repr";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let repr = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@repr requires a representation: packed, C, cache_optimal, transparent",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let repr_name = extract_ident(&args[0])?;
                match Repr::from_str(repr_name.as_str()) {
                    Maybe::Some(r) => r,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown repr '{}', expected: packed, C, cache_optimal, transparent",
                                repr_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@repr takes exactly one representation argument",
                    attr.span,
                ));
            }
        };

        Ok(ReprAttr::new(repr, attr.span))
    }
}

impl FromAttribute for PrefetchAttr {
    const NAME: &'static str = "prefetch";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut access = PrefetchAccess::Read;
        let mut locality: u8 = 3;

        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                // Check for read/write identifier
                if let Ok(name) = extract_ident(arg) {
                    if let Maybe::Some(a) = PrefetchAccess::from_str(name.as_str()) {
                        access = a;
                        continue;
                    }
                }

                // Check for locality: N
                if let Some(loc_expr) = extract_named_arg(std::slice::from_ref(arg), "locality") {
                    let val = extract_u32(loc_expr)?;
                    if val > 3 {
                        return Err(AttributeConversionError::invalid_args(
                            "locality must be 0-3",
                            loc_expr.span,
                        ));
                    }
                    locality = val as u8;
                }
            }
        }

        Ok(PrefetchAttr::new(access, locality, attr.span))
    }
}

impl FromAttribute for AccessPatternAttr {
    const NAME: &'static str = "access_pattern";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let pattern = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@access_pattern requires a pattern: sequential, random, streaming",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let pattern_name = extract_ident(&args[0])?;
                match AccessPattern::from_str(pattern_name.as_str()) {
                    Maybe::Some(p) => p,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown access pattern '{}', expected: sequential, random, streaming",
                                pattern_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@access_pattern takes exactly one pattern argument",
                    attr.span,
                ));
            }
        };

        Ok(AccessPatternAttr::new(pattern, attr.span))
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - VERIFICATION ATTRIBUTES
// =============================================================================

impl FromAttribute for VerifyAttr {
    const NAME: &'static str = "verify";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut modes = List::new();
        let mut timeout_ms = Maybe::None;

        match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@verify requires a mode: proof, static, runtime, assume",
                    attr.span,
                ));
            }
            Maybe::Some(args) => {
                for arg in args.iter() {
                    // Check for mode identifier
                    if let Ok(name) = extract_ident(arg) {
                        if let Maybe::Some(m) = VerificationMode::from_str(name.as_str()) {
                            modes.push(m);
                            continue;
                        }
                    }

                    // Check for array of modes: [proof, static, runtime]
                    if let ExprKind::Array(crate::expr::ArrayExpr::List(arr)) = &arg.kind {
                        for elem in arr.iter() {
                            let name = extract_ident(elem)?;
                            match VerificationMode::from_str(name.as_str()) {
                                Maybe::Some(m) => modes.push(m),
                                Maybe::None => {
                                    return Err(AttributeConversionError::invalid_args(
                                        format!(
                                            "unknown verification mode '{}', expected: proof, static, runtime, assume",
                                            name
                                        ),
                                        elem.span,
                                    ));
                                }
                            }
                        }
                        continue;
                    }

                    // Check for timeout: N
                    if let Some(timeout_expr) =
                        extract_named_arg(std::slice::from_ref(arg), "timeout")
                    {
                        timeout_ms = Maybe::Some(extract_u64(timeout_expr)?);
                    }
                }
            }
        }

        if modes.is_empty() {
            return Err(AttributeConversionError::invalid_args(
                "@verify requires at least one verification mode",
                attr.span,
            ));
        }

        let mut result = VerifyAttr::new(modes, attr.span);
        result.timeout_ms = timeout_ms;
        Ok(result)
    }
}

impl FromAttribute for AssumeAttr {
    const NAME: &'static str = "assume";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        match &attr.args {
            Maybe::None => {
                Err(AttributeConversionError::invalid_args(
                    "@assume requires a condition expression",
                    attr.span,
                ))
            }
            Maybe::Some(args) if args.len() == 1 => Ok(AssumeAttr::Condition {
                condition: args[0].clone(),
                span: attr.span,
            }),
            Maybe::Some(_) => {
                Err(AttributeConversionError::invalid_args(
                    "@assume takes exactly one condition expression",
                    attr.span,
                ))
            }
        }
    }
}

/// Handles @assume_no_alias attribute
impl FromAttribute for AssumeNoAliasAttr {
    const NAME: &'static str = "assume_no_alias";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@assume_no_alias takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(AssumeNoAliasAttr { span: attr.span })
    }
}

/// Wrapper for @assume_no_alias
#[derive(Debug, Clone, PartialEq)]
pub struct AssumeNoAliasAttr {
    pub span: Span,
}

impl AssumeNoAliasAttr {
    /// Convert to the general AssumeAttr
    pub fn to_assume_attr(self) -> AssumeAttr {
        AssumeAttr::NoAlias { span: self.span }
    }
}

/// Handles @assume_no_overflow attribute
#[derive(Debug, Clone, PartialEq)]
pub struct AssumeNoOverflowAttr {
    pub span: Span,
}

impl FromAttribute for AssumeNoOverflowAttr {
    const NAME: &'static str = "assume_no_overflow";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@assume_no_overflow takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(AssumeNoOverflowAttr { span: attr.span })
    }
}

impl AssumeNoOverflowAttr {
    /// Convert to the general AssumeAttr
    pub fn to_assume_attr(self) -> AssumeAttr {
        AssumeAttr::NoOverflow { span: self.span }
    }
}

/// Handles @assume_aligned attribute
#[derive(Debug, Clone, PartialEq)]
pub struct AssumeAlignedAttr {
    pub alignment: u32,
    pub span: Span,
}

impl FromAttribute for AssumeAlignedAttr {
    const NAME: &'static str = "assume_aligned";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let alignment = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@assume_aligned requires an alignment value",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let n = extract_u32(&args[0])?;
                if !n.is_power_of_two() {
                    return Err(AttributeConversionError::invalid_args(
                        "alignment must be a power of 2",
                        args[0].span,
                    ));
                }
                n
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@assume_aligned takes exactly one alignment argument",
                    attr.span,
                ));
            }
        };

        Ok(AssumeAlignedAttr {
            alignment,
            span: attr.span,
        })
    }
}

impl AssumeAlignedAttr {
    /// Convert to the general AssumeAttr
    pub fn to_assume_attr(self) -> AssumeAttr {
        AssumeAttr::Aligned {
            alignment: self.alignment,
            span: self.span,
        }
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - TARGET/CODEGEN ATTRIBUTES
// =============================================================================

impl FromAttribute for TargetCpuAttr {
    const NAME: &'static str = "target_cpu";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let cpu = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@target_cpu requires a CPU name: e.g., \"native\", \"x86-64-v3\"",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                // Accept either string or identifier
                if let Ok(s) = extract_string(&args[0]) {
                    s
                } else {
                    extract_ident(&args[0])?
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@target_cpu takes exactly one CPU name argument",
                    attr.span,
                ));
            }
        };

        Ok(TargetCpuAttr::new(cpu, attr.span))
    }
}

impl FromAttribute for TargetFeatureAttr {
    const NAME: &'static str = "target_feature";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let features = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@target_feature requires features: e.g., \"+avx2,+fma\"",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => extract_string(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@target_feature takes exactly one features string argument",
                    attr.span,
                ));
            }
        };

        Ok(TargetFeatureAttr::new(features, attr.span))
    }
}

impl FromAttribute for LtoAttr {
    const NAME: &'static str = "lto";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mode = match &attr.args {
            Maybe::None => LtoMode::Always, // Default to always when @lto is specified
            Maybe::Some(args) if args.is_empty() => LtoMode::Always,
            Maybe::Some(args) if args.len() == 1 => {
                let mode_name = extract_ident(&args[0])?;
                match mode_name.as_str() {
                    "always" => LtoMode::Always,
                    "thin" => LtoMode::Thin,
                    "none" => LtoMode::None,
                    _ => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown LTO mode '{}', expected: always, thin, none",
                                mode_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@lto takes at most one mode argument",
                    attr.span,
                ));
            }
        };

        Ok(LtoAttr::new(mode, attr.span))
    }
}

/// Handles @no_lto attribute
#[derive(Debug, Clone, PartialEq)]
pub struct NoLtoAttr {
    pub span: Span,
}

impl FromAttribute for NoLtoAttr {
    const NAME: &'static str = "no_lto";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@no_lto takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NoLtoAttr { span: attr.span })
    }
}

impl NoLtoAttr {
    /// Convert to LtoAttr with None mode
    pub fn to_lto_attr(self) -> LtoAttr {
        LtoAttr::new(LtoMode::None, self.span)
    }
}

impl FromAttribute for VisibilityAttr {
    const NAME: &'static str = "visibility";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let visibility = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@visibility requires a visibility level: hidden, default, protected",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let vis_name = extract_ident(&args[0])?;
                match SymbolVisibility::from_str(vis_name.as_str()) {
                    Maybe::Some(v) => v,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown visibility '{}', expected: hidden, default, protected",
                                vis_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@visibility takes exactly one visibility argument",
                    attr.span,
                ));
            }
        };

        Ok(VisibilityAttr::new(visibility, attr.span))
    }
}

// ============================================================================
// Linker Control Attributes (Phase 6)
// ============================================================================

impl FromAttribute for AliasAttr {
    const NAME: &'static str = "alias";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let target = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@alias requires a target symbol name",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                // Can be either an identifier or a string literal
                if let Ok(ident) = extract_ident(&args[0]) {
                    ident
                } else if let Ok(s) = extract_string(&args[0]) {
                    s
                } else {
                    return Err(AttributeConversionError::invalid_args(
                        "@alias target must be an identifier or string literal",
                        args[0].span,
                    ));
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@alias takes exactly one target argument",
                    attr.span,
                ));
            }
        };

        Ok(AliasAttr::new(target, attr.span))
    }
}

impl FromAttribute for WeakAttr {
    const NAME: &'static str = "weak";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@weak takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(WeakAttr::new(attr.span))
    }
}

impl FromAttribute for LinkageAttr {
    const NAME: &'static str = "linkage";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let kind = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@linkage requires a linkage kind: external, internal, private, weak, linkonce, linkonce_odr, common",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let kind_name = extract_ident(&args[0])?;
                match LinkageKind::from_str(kind_name.as_str()) {
                    Maybe::Some(k) => k,
                    Maybe::None => {
                        return Err(AttributeConversionError::invalid_args(
                            format!(
                                "unknown linkage kind '{}', expected: external, internal, private, weak, linkonce, linkonce_odr, common, available_externally",
                                kind_name
                            ),
                            args[0].span,
                        ));
                    }
                }
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@linkage takes exactly one linkage kind argument",
                    attr.span,
                ));
            }
        };

        Ok(LinkageAttr::new(kind, attr.span))
    }
}

impl FromAttribute for InitPriorityAttr {
    const NAME: &'static str = "init_priority";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let priority = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@init_priority requires a priority value (101-65535)",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let val = extract_int(&args[0])?;
                if !(101..=65535).contains(&val) {
                    return Err(AttributeConversionError::invalid_args(
                        format!(
                            "init_priority {} is out of range, must be 101-65535 (0-100 reserved for system)",
                            val
                        ),
                        args[0].span,
                    ));
                }
                val as u32
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@init_priority takes exactly one numeric argument",
                    attr.span,
                ));
            }
        };

        Ok(InitPriorityAttr::new(priority, attr.span))
    }
}

impl FromAttribute for SectionAttr {
    const NAME: &'static str = "section";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let name = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@section requires a section name string",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                extract_string(&args[0])?
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@section takes exactly one string argument",
                    attr.span,
                ));
            }
        };

        Ok(SectionAttr::new(name, attr.span))
    }
}

impl FromAttribute for ExportAttr {
    const NAME: &'static str = "export";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        match &attr.args {
            Maybe::None => Err(AttributeConversionError::invalid_args(
                "@export requires an ABI name (e.g., \"C\", \"stdcall\")",
                attr.span,
            )),
            Maybe::Some(args) if args.len() == 1 => {
                let abi = extract_string(&args[0])?;
                Ok(ExportAttr::new(abi, attr.span))
            }
            Maybe::Some(args) if args.len() == 2 => {
                // @export("C", name = "custom_name")
                let abi = extract_string(&args[0])?;
                // Second arg should be a named argument `name = "..."`
                // For now, accept a string as the export name
                let export_name = extract_string(&args[1])?;
                Ok(ExportAttr::with_name(abi, export_name, attr.span))
            }
            Maybe::Some(_) => Err(AttributeConversionError::invalid_args(
                "@export takes one or two arguments: @export(\"ABI\") or @export(\"ABI\", \"name\")",
                attr.span,
            )),
        }
    }
}

impl FromAttribute for UsedAttr {
    const NAME: &'static str = "used";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@used takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(UsedAttr::new(attr.span))
    }
}

impl FromAttribute for OptimizeBarrierAttr {
    const NAME: &'static str = "optimize_barrier";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@optimize_barrier takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(OptimizeBarrierAttr::new(attr.span))
    }
}

impl FromAttribute for BlackBoxAttr {
    const NAME: &'static str = "black_box";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@black_box takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(BlackBoxAttr::new(attr.span))
    }
}

impl FromAttribute for CpuDispatchAttr {
    const NAME: &'static str = "cpu_dispatch";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@cpu_dispatch takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(CpuDispatchAttr::new(attr.span))
    }
}

impl FromAttribute for MultiversionAttr {
    const NAME: &'static str = "multiversion";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut variants = List::new();

        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                // Look for named arguments like: avx512 = "avx512f,avx512vl"
                if let ExprKind::Binary { op, left, right } = &arg.kind {
                    if let crate::expr::BinOp::Assign = op {
                        let name = extract_ident(left)?;
                        let features = extract_string(right)?;
                        variants.push(MultiversionVariant::new(name, features, arg.span));
                    }
                }
            }
        }

        if variants.is_empty() {
            return Err(AttributeConversionError::invalid_args(
                "@multiversion requires at least one variant: @multiversion(name = \"features\", ...)",
                attr.span,
            ));
        }

        Ok(MultiversionAttr::new(variants, attr.span))
    }
}

impl FromAttribute for ConstEvalAttr {
    const NAME: &'static str = "const_eval";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        // @const_eval takes no arguments, defaults to Eval mode
        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@const_eval takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ConstEvalAttr::new(ConstEvalMode::Eval, attr.span))
    }
}

/// Handles @const_fold attribute
#[derive(Debug, Clone, PartialEq)]
pub struct ConstFoldAttr {
    pub span: Span,
}

impl FromAttribute for ConstFoldAttr {
    const NAME: &'static str = "const_fold";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@const_fold takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ConstFoldAttr { span: attr.span })
    }
}

impl ConstFoldAttr {
    /// Convert to ConstEvalAttr with Fold mode
    pub fn to_const_eval_attr(self) -> ConstEvalAttr {
        ConstEvalAttr::new(ConstEvalMode::Fold, self.span)
    }
}

/// Handles @const_prop attribute
#[derive(Debug, Clone, PartialEq)]
pub struct ConstPropAttr {
    pub span: Span,
}

impl FromAttribute for ConstPropAttr {
    const NAME: &'static str = "const_prop";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@const_prop takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ConstPropAttr { span: attr.span })
    }
}

impl ConstPropAttr {
    /// Convert to ConstEvalAttr with Propagate mode
    pub fn to_const_eval_attr(self) -> ConstEvalAttr {
        ConstEvalAttr::new(ConstEvalMode::Propagate, self.span)
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - LIKELIHOOD ATTRIBUTES
// =============================================================================

impl FromAttribute for LikelihoodAttr {
    const NAME: &'static str = "likely";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() == "likely" {
            if let Maybe::Some(args) = &attr.args {
                if !args.is_empty() {
                    return Err(AttributeConversionError::invalid_args(
                        "@likely takes no arguments",
                        attr.span,
                    ));
                }
            }
            return Ok(LikelihoodAttr::likely(attr.span));
        }

        if attr.name.as_str() == "unlikely" {
            if let Maybe::Some(args) = &attr.args {
                if !args.is_empty() {
                    return Err(AttributeConversionError::invalid_args(
                        "@unlikely takes no arguments",
                        attr.span,
                    ));
                }
            }
            return Ok(LikelihoodAttr::unlikely(attr.span));
        }

        Err(AttributeConversionError::wrong_name(
            Self::NAME,
            attr.name.as_str(),
            attr.span,
        ))
    }
}

/// Separate handler for @unlikely since FromAttribute::NAME can only have one value
#[derive(Debug, Clone, PartialEq)]
pub struct UnlikelyAttr {
    pub span: Span,
}

impl FromAttribute for UnlikelyAttr {
    const NAME: &'static str = "unlikely";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@unlikely takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(UnlikelyAttr { span: attr.span })
    }
}

impl UnlikelyAttr {
    /// Convert to LikelihoodAttr
    pub fn to_likelihood_attr(self) -> LikelihoodAttr {
        LikelihoodAttr::unlikely(self.span)
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - PGO ATTRIBUTES
// =============================================================================

impl FromAttribute for PgoAttr {
    const NAME: &'static str = "profile";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        // Handle @profile (for PGO, not the module profile)
        if attr.name.as_str() == "profile" {
            let name = if let Maybe::Some(args) = &attr.args {
                if let Some(name_expr) = extract_named_arg(args, "name") {
                    Maybe::Some(extract_string(name_expr)?)
                } else if args.len() == 1 {
                    Maybe::Some(extract_string(&args[0])?)
                } else if args.is_empty() {
                    Maybe::None
                } else {
                    return Err(AttributeConversionError::invalid_args(
                        "@profile takes at most one name argument",
                        attr.span,
                    ));
                }
            } else {
                Maybe::None
            };

            return Ok(PgoAttr::Profile {
                name,
                span: attr.span,
            });
        }

        Err(AttributeConversionError::wrong_name(
            Self::NAME,
            attr.name.as_str(),
            attr.span,
        ))
    }
}

/// Handles @frequency attribute
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyAttr {
    pub calls_per_sec: u64,
    pub span: Span,
}

impl FromAttribute for FrequencyAttr {
    const NAME: &'static str = "frequency";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let calls_per_sec = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@frequency requires a calls per second value",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => extract_u64(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@frequency takes exactly one numeric argument",
                    attr.span,
                ));
            }
        };

        Ok(FrequencyAttr {
            calls_per_sec,
            span: attr.span,
        })
    }
}

impl FrequencyAttr {
    /// Convert to PgoAttr
    pub fn to_pgo_attr(self) -> PgoAttr {
        PgoAttr::Frequency {
            calls_per_sec: self.calls_per_sec,
            span: self.span,
        }
    }
}

/// Handles @branch_probability attribute
#[derive(Debug, Clone, PartialEq)]
pub struct BranchProbabilityAttr {
    pub probability: f64,
    pub span: Span,
}

impl FromAttribute for BranchProbabilityAttr {
    const NAME: &'static str = "branch_probability";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let probability = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@branch_probability requires a probability value (0.0-1.0)",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => {
                let p = extract_float(&args[0])?;
                if !(0.0..=1.0).contains(&p) {
                    return Err(AttributeConversionError::invalid_args(
                        "probability must be between 0.0 and 1.0",
                        args[0].span,
                    ));
                }
                p
            }
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@branch_probability takes exactly one probability argument",
                    attr.span,
                ));
            }
        };

        Ok(BranchProbabilityAttr {
            probability,
            span: attr.span,
        })
    }
}

impl BranchProbabilityAttr {
    /// Convert to PgoAttr
    pub fn to_pgo_attr(self) -> PgoAttr {
        PgoAttr::BranchProbability {
            probability: self.probability,
            span: self.span,
        }
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - PERFORMANCE CONTRACT ATTRIBUTES
// =============================================================================

/// Handles @constant_time attribute
#[derive(Debug, Clone, PartialEq)]
pub struct ConstantTimeAttr {
    pub span: Span,
}

impl FromAttribute for ConstantTimeAttr {
    const NAME: &'static str = "constant_time";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@constant_time takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(ConstantTimeAttr { span: attr.span })
    }
}

impl ConstantTimeAttr {
    /// Convert to PerformanceContract
    pub fn to_performance_contract(self) -> PerformanceContract {
        PerformanceContract::ConstantTime { span: self.span }
    }
}

/// Handles @max_time attribute
#[derive(Debug, Clone, PartialEq)]
pub struct MaxTimeAttr {
    pub microseconds: u64,
    pub span: Span,
}

impl FromAttribute for MaxTimeAttr {
    const NAME: &'static str = "max_time";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let microseconds = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@max_time requires a duration in microseconds",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => extract_u64(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@max_time takes exactly one duration argument",
                    attr.span,
                ));
            }
        };

        Ok(MaxTimeAttr {
            microseconds,
            span: attr.span,
        })
    }
}

impl MaxTimeAttr {
    /// Convert to PerformanceContract
    pub fn to_performance_contract(self) -> PerformanceContract {
        PerformanceContract::MaxTime {
            microseconds: self.microseconds,
            span: self.span,
        }
    }
}

/// Handles @max_memory attribute
#[derive(Debug, Clone, PartialEq)]
pub struct MaxMemoryAttr {
    pub bytes: u64,
    pub span: Span,
}

impl FromAttribute for MaxMemoryAttr {
    const NAME: &'static str = "max_memory";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let bytes = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@max_memory requires a size in bytes",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => extract_u64(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@max_memory takes exactly one size argument",
                    attr.span,
                ));
            }
        };

        Ok(MaxMemoryAttr {
            bytes,
            span: attr.span,
        })
    }
}

impl MaxMemoryAttr {
    /// Convert to PerformanceContract
    pub fn to_performance_contract(self) -> PerformanceContract {
        PerformanceContract::MaxMemory {
            bytes: self.bytes,
            span: self.span,
        }
    }
}

// =============================================================================
// FromAttribute IMPLEMENTATIONS - META-SYSTEM ATTRIBUTES
// =============================================================================

impl FromAttribute for TaggedLiteralAttr {
    const NAME: &'static str = "tagged_literal";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let tag = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@tagged_literal requires a tag name: @tagged_literal(\"json\")",
                    attr.span,
                ));
            }
            Maybe::Some(args) if !args.is_empty() => extract_string(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@tagged_literal requires at least a tag name",
                    attr.span,
                ));
            }
        };

        Ok(TaggedLiteralAttr::new(tag, attr.span))
    }
}

impl FromAttribute for DifferentiableAttr {
    const NAME: &'static str = "differentiable";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut wrt = List::new();
        let mut mode = Text::from("reverse");
        let mut custom_vjp = Maybe::None;

        if let Maybe::Some(args) = &attr.args {
            // Look for wrt = "params" or wrt = ["param1", "param2"]
            if let Some(wrt_expr) = extract_named_arg(args, "wrt") {
                // Could be a string or array
                if let Ok(s) = extract_string(wrt_expr) {
                    // Split by comma
                    for part in s.as_str().split(',') {
                        wrt.push(Text::from(part.trim()));
                    }
                } else if let Ok(list) = extract_string_list(wrt_expr) {
                    wrt = list;
                }
            }

            if let Some(mode_expr) = extract_named_arg(args, "mode") {
                mode = extract_string(mode_expr)?;
            }

            if let Some(vjp_expr) = extract_named_arg(args, "custom_vjp") {
                custom_vjp = Maybe::Some(extract_string(vjp_expr)?);
            }
        }

        let mut result = DifferentiableAttr::new(wrt, attr.span);
        result.mode = mode;
        result.custom_vjp = custom_vjp;
        Ok(result)
    }
}

// =============================================================================
// SIMD/VECTORIZE VARIANT ATTRIBUTES
// =============================================================================

/// Handles @simd attribute (alias for @vectorize)
#[derive(Debug, Clone, PartialEq)]
pub struct SimdAttr {
    pub mode: VectorizeMode,
    pub width: Maybe<u32>,
    pub span: Span,
}

impl FromAttribute for SimdAttr {
    const NAME: &'static str = "simd";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let mut mode = VectorizeMode::Auto;
        let mut width = Maybe::None;

        if let Maybe::Some(args) = &attr.args {
            for arg in args.iter() {
                if let Ok(name) = extract_ident(arg) {
                    match name.as_str() {
                        "prefer" => mode = VectorizeMode::Prefer,
                        "force" => mode = VectorizeMode::Force,
                        "never" => mode = VectorizeMode::Never,
                        _ => {}
                    }
                }

                if let Some(width_expr) = extract_named_arg(std::slice::from_ref(arg), "width") {
                    width = Maybe::Some(extract_u32(width_expr)?);
                }
            }
        }

        Ok(SimdAttr {
            mode,
            width,
            span: attr.span,
        })
    }
}

impl SimdAttr {
    /// Convert to VectorizeAttr
    pub fn to_vectorize_attr(self) -> VectorizeAttr {
        let mut attr = VectorizeAttr::new(self.mode, self.span);
        attr.width = self.width;
        attr
    }
}

/// Handles @no_vectorize attribute
#[derive(Debug, Clone, PartialEq)]
pub struct NoVectorizeAttr {
    pub span: Span,
}

impl FromAttribute for NoVectorizeAttr {
    const NAME: &'static str = "no_vectorize";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@no_vectorize takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NoVectorizeAttr { span: attr.span })
    }
}

impl NoVectorizeAttr {
    /// Convert to VectorizeAttr with Never mode
    pub fn to_vectorize_attr(self) -> VectorizeAttr {
        VectorizeAttr::new(VectorizeMode::Never, self.span)
    }
}

// =============================================================================
// ADDITIONAL LINKER CONTROL ATTRIBUTES
// =============================================================================

impl FromAttribute for NakedAttr {
    const NAME: &'static str = "naked";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@naked takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NakedAttr::new(attr.span))
    }
}

impl FromAttribute for LinkNameAttr {
    const NAME: &'static str = "link_name";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let name = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@link_name requires a symbol name string",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => extract_string(&args[0])?,
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@link_name takes exactly one string argument",
                    attr.span,
                ));
            }
        };

        Ok(LinkNameAttr::new(name, attr.span))
    }
}

impl FromAttribute for NoReturnAttr {
    const NAME: &'static str = "noreturn";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@noreturn takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NoReturnAttr::new(attr.span))
    }
}

impl FromAttribute for NoMangleAttr {
    const NAME: &'static str = "no_mangle";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@no_mangle takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(NoMangleAttr::new(attr.span))
    }
}

impl FromAttribute for LlvmOnlyAttr {
    const NAME: &'static str = "llvm_only";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let reason = match &attr.args {
            Maybe::None => Maybe::None,
            Maybe::Some(args) if args.is_empty() => Maybe::None,
            Maybe::Some(args) => {
                // Look for reason = "..." argument
                if let Some(reason_expr) = extract_named_arg(args, "reason") {
                    Maybe::Some(extract_string(reason_expr)?)
                } else if args.len() == 1 {
                    // Allow @llvm_only("reason") shorthand
                    Maybe::Some(extract_string(&args[0])?)
                } else {
                    return Err(AttributeConversionError::invalid_args(
                        "@llvm_only takes at most one reason argument: @llvm_only(reason = \"...\")",
                        attr.span,
                    ));
                }
            }
        };

        match reason {
            Maybe::Some(r) => Ok(LlvmOnlyAttr::with_reason(r, attr.span)),
            Maybe::None => Ok(LlvmOnlyAttr::new(attr.span)),
        }
    }
}

// =============================================================================
// FORMAL VERIFICATION ATTRIBUTES
// =============================================================================

impl FromAttribute for GhostAttr {
    const NAME: &'static str = "ghost";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        if let Maybe::Some(args) = &attr.args {
            if !args.is_empty() {
                return Err(AttributeConversionError::invalid_args(
                    "@ghost takes no arguments",
                    attr.span,
                ));
            }
        }

        Ok(GhostAttr::new(attr.span))
    }
}

impl FromAttribute for RequiresAttr {
    const NAME: &'static str = "requires";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let condition = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@requires requires a condition expression",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => args[0].clone(),
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@requires takes exactly one condition expression",
                    attr.span,
                ));
            }
        };

        Ok(RequiresAttr::new(condition, attr.span))
    }
}

impl FromAttribute for EnsuresAttr {
    const NAME: &'static str = "ensures";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let condition = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@ensures requires a condition expression",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => args[0].clone(),
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@ensures takes exactly one condition expression",
                    attr.span,
                ));
            }
        };

        Ok(EnsuresAttr::new(condition, attr.span))
    }
}

impl FromAttribute for InvariantAttr {
    const NAME: &'static str = "invariant";

    fn from_attribute(attr: &Attribute) -> Result<Self, AttributeConversionError> {
        if attr.name.as_str() != Self::NAME {
            return Err(AttributeConversionError::wrong_name(
                Self::NAME,
                attr.name.as_str(),
                attr.span,
            ));
        }

        let condition = match &attr.args {
            Maybe::None => {
                return Err(AttributeConversionError::invalid_args(
                    "@invariant requires a condition expression",
                    attr.span,
                ));
            }
            Maybe::Some(args) if args.len() == 1 => args[0].clone(),
            Maybe::Some(_) => {
                return Err(AttributeConversionError::invalid_args(
                    "@invariant takes exactly one condition expression",
                    attr.span,
                ));
            }
        };

        Ok(InvariantAttr::new(condition, attr.span))
    }
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ident;
    use crate::literal::Literal;

    fn make_attr(name: &str, args: Option<Vec<Expr>>) -> Attribute {
        Attribute::new(
            Text::from(name),
            args.map(|a| a.into()),
            Span::default(),
        )
    }

    fn ident_expr(name: &str) -> Expr {
        Expr::ident(Ident::new(name, Span::default()))
    }

    fn int_expr(value: i128) -> Expr {
        Expr::literal(Literal::int(value, Span::default()))
    }

    fn string_expr(value: &str) -> Expr {
        Expr::literal(Literal::string(Text::from(value), Span::default()))
    }

    #[test]
    fn test_inline_attr_no_args() {
        let attr = make_attr("inline", None);
        let result = InlineAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, InlineMode::Suggest);
    }

    #[test]
    fn test_inline_attr_always() {
        let attr = make_attr("inline", Some(vec![ident_expr("always")]));
        let result = InlineAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, InlineMode::Always);
    }

    #[test]
    fn test_inline_attr_never() {
        let attr = make_attr("inline", Some(vec![ident_expr("never")]));
        let result = InlineAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, InlineMode::Never);
    }

    #[test]
    fn test_inline_attr_release() {
        let attr = make_attr("inline", Some(vec![ident_expr("release")]));
        let result = InlineAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, InlineMode::Release);
    }

    #[test]
    fn test_inline_attr_invalid_mode() {
        let attr = make_attr("inline", Some(vec![ident_expr("invalid")]));
        let result = InlineAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_cold_attr() {
        let attr = make_attr("cold", None);
        let result = ColdAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cold_attr_with_args_fails() {
        let attr = make_attr("cold", Some(vec![ident_expr("foo")]));
        let result = ColdAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_hot_attr() {
        let attr = make_attr("hot", None);
        let result = HotAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_optimize_attr_speed() {
        let attr = make_attr("optimize", Some(vec![ident_expr("speed")]));
        let result = OptimizeAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.level, OptimizationLevel::Speed);
    }

    #[test]
    fn test_optimize_attr_size() {
        let attr = make_attr("optimize", Some(vec![ident_expr("size")]));
        let result = OptimizeAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.level, OptimizationLevel::Size);
    }

    #[test]
    fn test_align_attr() {
        let attr = make_attr("align", Some(vec![int_expr(32)]));
        let result = AlignAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.alignment, 32);
    }

    #[test]
    fn test_align_attr_non_power_of_two_fails() {
        let attr = make_attr("align", Some(vec![int_expr(17)]));
        let result = AlignAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_repr_attr_c() {
        let attr = make_attr("repr", Some(vec![ident_expr("C")]));
        let result = ReprAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.repr, Repr::C);
    }

    #[test]
    fn test_repr_attr_packed() {
        let attr = make_attr("repr", Some(vec![ident_expr("packed")]));
        let result = ReprAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.repr, Repr::Packed);
    }

    #[test]
    fn test_unroll_attr_count() {
        let attr = make_attr("unroll", Some(vec![int_expr(4)]));
        let result = UnrollAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, UnrollMode::Count(4));
    }

    #[test]
    fn test_unroll_attr_full() {
        let attr = make_attr("unroll", Some(vec![ident_expr("full")]));
        let result = UnrollAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, UnrollMode::Full);
    }

    #[test]
    fn test_vectorize_attr_force() {
        let attr = make_attr("vectorize", Some(vec![ident_expr("force")]));
        let result = VectorizeAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, VectorizeMode::Force);
    }

    #[test]
    fn test_lock_level_attr() {
        let attr = make_attr("lock_level", Some(vec![int_expr(5)]));
        let result = LockLevelAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.level, 5);
    }

    #[test]
    fn test_target_cpu_attr() {
        let attr = make_attr("target_cpu", Some(vec![string_expr("native")]));
        let result = TargetCpuAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.cpu.as_str(), "native");
    }

    #[test]
    fn test_target_feature_attr() {
        let attr = make_attr("target_feature", Some(vec![string_expr("+avx2,+fma")]));
        let result = TargetFeatureAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.features.as_str(), "+avx2,+fma");
    }

    #[test]
    fn test_used_attr() {
        let attr = make_attr("used", None);
        let result = UsedAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_visibility_attr_hidden() {
        let attr = make_attr("visibility", Some(vec![ident_expr("hidden")]));
        let result = VisibilityAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.visibility, SymbolVisibility::Hidden);
    }

    #[test]
    fn test_parallel_attr() {
        let attr = make_attr("parallel", None);
        let result = ParallelAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_alias_attr() {
        let attr = make_attr("no_alias", None);
        let result = NoAliasAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_ivdep_attr() {
        let attr = make_attr("ivdep", None);
        let result = IvdepAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_optimize_barrier_attr() {
        let attr = make_attr("optimize_barrier", None);
        let result = OptimizeBarrierAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_black_box_attr() {
        let attr = make_attr("black_box", None);
        let result = BlackBoxAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cpu_dispatch_attr() {
        let attr = make_attr("cpu_dispatch", None);
        let result = CpuDispatchAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_const_eval_attr() {
        let attr = make_attr("const_eval", None);
        let result = ConstEvalAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, ConstEvalMode::Eval);
    }

    #[test]
    fn test_lto_attr_always() {
        let attr = make_attr("lto", Some(vec![ident_expr("always")]));
        let result = LtoAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, LtoMode::Always);
    }

    #[test]
    fn test_lto_attr_thin() {
        let attr = make_attr("lto", Some(vec![ident_expr("thin")]));
        let result = LtoAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, LtoMode::Thin);
    }

    #[test]
    fn test_lto_attr_default() {
        let attr = make_attr("lto", None);
        let result = LtoAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.mode, LtoMode::Always);
    }

    #[test]
    fn test_std_attr_no_args() {
        let attr = make_attr("std", None);
        let result = StdAttr::from_attribute(&attr).unwrap();
        assert!(result.context_group.is_none());
    }

    #[test]
    fn test_std_attr_with_context() {
        let attr = make_attr("std", Some(vec![ident_expr("ServerContext")]));
        let result = StdAttr::from_attribute(&attr).unwrap();
        assert!(result.context_group.is_some());
    }

    #[test]
    fn test_profile_attr_application() {
        let attr = make_attr("profile", Some(vec![ident_expr("application")]));
        let result = ProfileAttr::from_attribute(&attr).unwrap();
        assert!(result.profiles.contains(&Profile::Application));
    }

    #[test]
    fn test_profile_attr_multiple() {
        let attr = make_attr(
            "profile",
            Some(vec![ident_expr("systems"), ident_expr("research")]),
        );
        let result = ProfileAttr::from_attribute(&attr).unwrap();
        assert!(result.profiles.contains(&Profile::Systems));
        assert!(result.profiles.contains(&Profile::Research));
    }

    #[test]
    fn test_reduce_attr_add() {
        let attr = make_attr("reduce", Some(vec![ident_expr("+")]));
        let result = ReduceAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.op, ReductionOp::Add);
    }

    #[test]
    fn test_access_pattern_attr_sequential() {
        let attr = make_attr("access_pattern", Some(vec![ident_expr("sequential")]));
        let result = AccessPatternAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.pattern, AccessPattern::Sequential);
    }

    #[test]
    fn test_specialize_attr_basic() {
        let attr = make_attr("specialize", None);
        let result = SpecializeAttr::from_attribute(&attr).unwrap();
        assert!(!result.negative);
        assert!(result.rank.is_none());
    }

    #[test]
    fn test_specialize_attr_negative() {
        let attr = make_attr("specialize", Some(vec![ident_expr("negative")]));
        let result = SpecializeAttr::from_attribute(&attr).unwrap();
        assert!(result.negative);
    }

    #[test]
    fn test_tagged_literal_attr() {
        let attr = make_attr("tagged_literal", Some(vec![string_expr("json")]));
        let result = TaggedLiteralAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.tag.as_str(), "json");
    }

    #[test]
    fn test_constant_time_attr() {
        let attr = make_attr("constant_time", None);
        let result = ConstantTimeAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_time_attr() {
        let attr = make_attr("max_time", Some(vec![int_expr(100)]));
        let result = MaxTimeAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.microseconds, 100);
    }

    #[test]
    fn test_max_memory_attr() {
        let attr = make_attr("max_memory", Some(vec![int_expr(1024)]));
        let result = MaxMemoryAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.bytes, 1024);
    }

    #[test]
    fn test_frequency_attr() {
        let attr = make_attr("frequency", Some(vec![int_expr(1000)]));
        let result = FrequencyAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.calls_per_sec, 1000);
    }

    #[test]
    fn test_wrong_name_error() {
        let attr = make_attr("cold", None);
        let result = HotAttr::from_attribute(&attr);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.message.as_str().contains("expected @hot"));
        }
    }

    // =========================================================================
    // Additional Linker Control Attribute Tests
    // =========================================================================

    #[test]
    fn test_naked_attr() {
        let attr = make_attr("naked", None);
        let result = NakedAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_naked_attr_rejects_args() {
        let attr = make_attr("naked", Some(vec![ident_expr("foo")]));
        let result = NakedAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_link_name_attr() {
        let attr = make_attr("link_name", Some(vec![string_expr("_my_symbol")]));
        let result = LinkNameAttr::from_attribute(&attr).unwrap();
        assert_eq!(result.name.as_str(), "_my_symbol");
    }

    #[test]
    fn test_link_name_attr_requires_arg() {
        let attr = make_attr("link_name", None);
        let result = LinkNameAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_noreturn_attr() {
        let attr = make_attr("noreturn", None);
        let result = NoReturnAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_no_mangle_attr() {
        let attr = make_attr("no_mangle", None);
        let result = NoMangleAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    // =========================================================================
    // Formal Verification Attribute Tests
    // =========================================================================

    #[test]
    fn test_ghost_attr() {
        let attr = make_attr("ghost", None);
        let result = GhostAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_requires_attr() {
        let attr = make_attr("requires", Some(vec![ident_expr("x > 0")]));
        let result = RequiresAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_requires_attr_requires_arg() {
        let attr = make_attr("requires", None);
        let result = RequiresAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_ensures_attr() {
        let attr = make_attr("ensures", Some(vec![ident_expr("result >= 0")]));
        let result = EnsuresAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_ensures_attr_requires_arg() {
        let attr = make_attr("ensures", None);
        let result = EnsuresAttr::from_attribute(&attr);
        assert!(result.is_err());
    }

    #[test]
    fn test_invariant_attr() {
        let attr = make_attr("invariant", Some(vec![ident_expr("len <= capacity")]));
        let result = InvariantAttr::from_attribute(&attr);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invariant_attr_requires_arg() {
        let attr = make_attr("invariant", None);
        let result = InvariantAttr::from_attribute(&attr);
        assert!(result.is_err());
    }
}
