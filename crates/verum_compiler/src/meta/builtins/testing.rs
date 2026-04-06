//! Testing Builtins for Meta-System Error Validation
//!
//! This module provides builtin functions that trigger specific meta error
//! codes for testing purposes. These are Tier 0 functions (always available).
//!
//! ## Functions
//!
//! - `trigger_type_reduction_failed(ty, msg)` - Trigger M501 TypeReductionFailed
//! - `trigger_normalization_diverged(ty, iters)` - Trigger M502 NormalizationDiverged
//! - `trigger_smt_verification_failed(constraint, reason)` - Trigger M503 SMTVerificationFailed
//! - `trigger_proof_construction_failed(goal, msg)` - Trigger M504 ProofConstructionFailed
//! - `trigger_refinement_violation(predicate, value)` - Trigger M505 RefinementViolation
//! - `trigger_meta_where_unsatisfied(constraint)` - Trigger M506 MetaWhereUnsatisfied
//!
//! These functions are primarily for testing the error reporting infrastructure.
//! They allow test cases to verify that error codes are properly propagated.
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_common::{List, Text};

use crate::meta::context::{ConstValue, MetaContext};
use crate::meta::MetaError;

use super::context_requirements::{BuiltinInfo, BuiltinRegistry};

/// Register all testing builtins
pub fn register_builtins(map: &mut BuiltinRegistry) {
    // M501 - TypeReductionFailed
    map.insert(
        Text::from("trigger_type_reduction_failed"),
        BuiltinInfo::tier0(
            trigger_type_reduction_failed,
            "Trigger M501 TypeReductionFailed error for testing",
            "(ty: Text, message: Text) -> never",
        ),
    );

    // M502 - NormalizationDiverged
    map.insert(
        Text::from("trigger_normalization_diverged"),
        BuiltinInfo::tier0(
            trigger_normalization_diverged,
            "Trigger M502 NormalizationDiverged error for testing",
            "(ty: Text, iterations: Int) -> never",
        ),
    );

    // M503 - SMTVerificationFailed
    map.insert(
        Text::from("trigger_smt_verification_failed"),
        BuiltinInfo::tier0(
            trigger_smt_verification_failed,
            "Trigger M503 SMTVerificationFailed error for testing",
            "(constraint: Text, reason: Text) -> never",
        ),
    );

    // M504 - ProofConstructionFailed
    map.insert(
        Text::from("trigger_proof_construction_failed"),
        BuiltinInfo::tier0(
            trigger_proof_construction_failed,
            "Trigger M504 ProofConstructionFailed error for testing",
            "(goal: Text, message: Text) -> never",
        ),
    );

    // M505 - RefinementViolation
    map.insert(
        Text::from("trigger_refinement_violation"),
        BuiltinInfo::tier0(
            trigger_refinement_violation,
            "Trigger M505 RefinementViolation error for testing",
            "(predicate: Text, value: Text) -> never",
        ),
    );

    // M506 - MetaWhereUnsatisfied
    map.insert(
        Text::from("trigger_meta_where_unsatisfied"),
        BuiltinInfo::tier0(
            trigger_meta_where_unsatisfied,
            "Trigger M506 MetaWhereUnsatisfied error for testing",
            "(constraint: Text) -> never",
        ),
    );
}

/// Trigger M501 TypeReductionFailed
fn trigger_type_reduction_failed(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let ty = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    let message = match args.get(1) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("type reduction failed"),
    };
    Err(MetaError::TypeReductionFailed { ty, message })
}

/// Trigger M502 NormalizationDiverged
fn trigger_normalization_diverged(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let ty = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    let iterations = match args.get(1) {
        Some(ConstValue::Int(n)) => *n as usize,
        _ => 10000,
    };
    Err(MetaError::NormalizationDiverged { ty, iterations })
}

/// Trigger M503 SMTVerificationFailed
fn trigger_smt_verification_failed(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let constraint = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    let reason = match args.get(1) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("verification failed"),
    };
    Err(MetaError::SMTVerificationFailed { constraint, reason })
}

/// Trigger M504 ProofConstructionFailed
fn trigger_proof_construction_failed(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let goal = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    let message = match args.get(1) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("proof construction failed"),
    };
    Err(MetaError::ProofConstructionFailed { goal, message })
}

/// Trigger M505 RefinementViolation
fn trigger_refinement_violation(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let predicate = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    let value = match args.get(1) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    Err(MetaError::RefinementViolation { predicate, value })
}

/// Trigger M506 MetaWhereUnsatisfied
fn trigger_meta_where_unsatisfied(
    _ctx: &mut MetaContext,
    args: List<ConstValue>,
) -> Result<ConstValue, MetaError> {
    let constraint = match args.get(0) {
        Some(ConstValue::Text(s)) => s.clone(),
        _ => Text::from("unknown"),
    };
    Err(MetaError::MetaWhereUnsatisfied { constraint })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trigger_type_reduction_failed() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("TypeFamily<Int>")),
            ConstValue::Text(Text::from("no matching case")),
        ]);

        let result = trigger_type_reduction_failed(&mut ctx, args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code(), "M501");
    }

    #[test]
    fn test_trigger_refinement_violation() {
        let mut ctx = MetaContext::new();
        let args = List::from(vec![
            ConstValue::Text(Text::from("x > 0")),
            ConstValue::Text(Text::from("-5")),
        ]);

        let result = trigger_refinement_violation(&mut ctx, args);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code(), "M505");
    }
}
