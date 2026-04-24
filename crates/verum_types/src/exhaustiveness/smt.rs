//! SMT Guard Verification — Types and Trait
//!
//! Keeps the pure *data types* used for guard verification (pattern,
//! configuration, result, witness, value) plus a trait boundary
//! `GuardVerifier`. The Z3-based implementation (`SmtGuardVerifier`) lives
//! in `verum_smt::exhaustiveness_backend` to avoid cyclic dependencies.
//!
//! ## Integration with Exhaustiveness
//!
//! The main exhaustiveness checker treats guards conservatively (as potentially failing).
//! This module defines the data types consumed by:
//! - A match has only guarded arms (E0603 warning candidate)
//! - Guards use arithmetic that can be proven exhaustive
//! - Guards are demonstrably redundant via SMT
//!
//! Callers obtain an SMT-backed verifier from `verum_smt` and pass it as
//! `&dyn GuardVerifier` to `check_exhaustiveness_with_options`.

use super::matrix::PatternColumn;
use crate::context::TypeEnv;
use crate::ty::Type;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use verum_ast::expr::Expr;
use verum_common::{List, Text};

/// Configuration for SMT-backed guard verification
#[derive(Debug, Clone)]
pub struct SmtGuardConfig {
    /// Timeout for individual guard checks (default: 100ms)
    pub timeout_ms: u64,
    /// Maximum number of guards to analyze with SMT (default: 10)
    pub max_guards: usize,
    /// Enable witness extraction for uncovered cases
    pub extract_witnesses: bool,
    /// Enable guard redundancy detection
    pub detect_redundancy: bool,
}

impl Default for SmtGuardConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 100,
            max_guards: 10,
            extract_witnesses: true,
            detect_redundancy: true,
        }
    }
}

/// Result of SMT guard verification
#[derive(Debug, Clone)]
pub struct SmtGuardResult {
    /// Whether all guards together are provably exhaustive
    pub is_exhaustive: bool,
    /// Indices of redundant guards (covered by earlier guards)
    pub redundant_guards: List<usize>,
    /// Witness values for uncovered cases (if any)
    pub uncovered_witnesses: List<SmtWitness>,
    /// Guards that couldn't be analyzed (too complex for SMT)
    pub unknown_guards: List<usize>,
    /// Time spent in SMT solving
    pub solve_time: Duration,
    /// Whether SMT analysis was skipped (too many guards, etc.)
    pub skipped: bool,
    /// Reason for skipping, if applicable
    pub skip_reason: Option<Text>,
}

impl SmtGuardResult {
    /// Create a result for when SMT analysis is skipped
    pub fn skipped(reason: impl Into<Text>) -> Self {
        Self {
            is_exhaustive: false,
            redundant_guards: List::new(),
            uncovered_witnesses: List::new(),
            unknown_guards: List::new(),
            solve_time: Duration::ZERO,
            skipped: true,
            skip_reason: Some(reason.into()),
        }
    }

    /// Create an empty result
    pub fn empty() -> Self {
        Self {
            is_exhaustive: false,
            redundant_guards: List::new(),
            uncovered_witnesses: List::new(),
            unknown_guards: List::new(),
            solve_time: Duration::ZERO,
            skipped: false,
            skip_reason: None,
        }
    }
}

/// A witness value extracted from SMT model
#[derive(Debug, Clone)]
pub struct SmtWitness {
    /// Variable name -> value mapping
    pub bindings: HashMap<Text, SmtValue>,
    /// Human-readable description
    pub description: Text,
}

/// Concrete value from SMT model
#[derive(Debug, Clone)]
pub enum SmtValue {
    Int(i128),
    Float(f64),
    Bool(bool),
    Unknown,
}

impl std::fmt::Display for SmtValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SmtValue::Int(n) => write!(f, "{}", n),
            SmtValue::Float(n) => write!(f, "{}", n),
            SmtValue::Bool(b) => write!(f, "{}", b),
            SmtValue::Unknown => write!(f, "_"),
        }
    }
}

/// Guard expression with its pattern context
#[derive(Debug, Clone)]
pub struct GuardedPattern {
    /// Index in the original pattern list
    pub pattern_index: usize,
    /// The base pattern (without guard)
    pub base_pattern: PatternColumn,
    /// The guard expression
    pub guard: Arc<Expr>,
    /// Variables bound by the pattern
    pub bound_vars: HashMap<Text, Type>,
}

/// Trait implemented by any guard verifier (SMT-backed or otherwise).
///
/// `verum_types` exposes the interface; concrete SMT-backed implementations
/// live in `verum_smt` to keep the dependency edge `verum_smt →
/// verum_types` one-way.
pub trait GuardVerifier: Send + Sync {
    /// Verify whether guarded patterns are exhaustive for a type.
    ///
    /// Returns:
    /// - Whether all guards together cover all possible values
    /// - Which guards are redundant
    /// - Witnesses for uncovered cases (if any)
    fn verify_guards(
        &self,
        patterns: &[GuardedPattern],
        scrutinee_ty: &Type,
        env: &TypeEnv,
    ) -> SmtGuardResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smt_config_default() {
        let config = SmtGuardConfig::default();
        assert_eq!(config.timeout_ms, 100);
        assert_eq!(config.max_guards, 10);
        assert!(config.extract_witnesses);
        assert!(config.detect_redundancy);
    }

    #[test]
    fn test_smt_result_skipped() {
        let result = SmtGuardResult::skipped("test reason");
        assert!(result.skipped);
        assert_eq!(result.skip_reason, Some(Text::from("test reason")));
        assert!(!result.is_exhaustive);
    }

    #[test]
    fn test_smt_value_display() {
        assert_eq!(format!("{}", SmtValue::Int(42)), "42");
        assert_eq!(format!("{}", SmtValue::Bool(true)), "true");
        assert_eq!(format!("{}", SmtValue::Unknown), "_");
    }
}
