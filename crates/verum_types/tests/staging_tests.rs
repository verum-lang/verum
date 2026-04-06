#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
//! Tests for multi-stage metaprogramming (staging) validation.
//!
//! Covers:
//! - Stage coherence: quote generates correct stage
//! - Cross-stage call restrictions
//! - Stage 0 cannot reference stage 1+ values
//! - Quote/unquote type correctness
//! - Stage overflow, cyclic dependencies, escape validation
//! - Warnings: unused stages, stage downgrades

use verum_ast::{FileId, Span};
use verum_common::{List, Text};
use verum_types::stage_checker::{
    FunctionStageInfo, StageChecker, StageConfig, StageError, StageWarning,
};

fn test_span() -> Span {
    Span::new(0, 10, FileId::new(0))
}

// =============================================================================
// STAGE COHERENCE RULE
// =============================================================================

#[test]
fn test_stage_1_generates_stage_0() {
    // meta fn generates runtime code -- valid
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("gen_runtime"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("gen_runtime"), 1, test_span());

    assert!(checker.check_quote(None, test_span()).is_ok());
    assert!(checker.check_quote(Some(0), test_span()).is_ok());
}

#[test]
fn test_stage_2_generates_stage_1() {
    // meta(2) fn generates meta(1) code -- valid
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("gen_meta"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("gen_meta"), 2, test_span());

    assert!(checker.check_quote(None, test_span()).is_ok());
    assert!(checker.check_quote(Some(1), test_span()).is_ok());
}

#[test]
fn test_stage_2_cannot_generate_stage_0_directly() {
    // meta(2) fn cannot directly generate stage 0 code
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("bad_gen"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("bad_gen"), 2, test_span());

    let result = checker.check_quote(Some(0), test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));

    if let Err(StageError::StageMismatch {
        current_stage,
        target_stage,
        expected_stage,
        ..
    }) = result
    {
        assert_eq!(current_stage, 2);
        assert_eq!(target_stage, 0);
        assert_eq!(expected_stage, 1);
    }
}

#[test]
fn test_quote_in_runtime_code_is_error() {
    // quote in stage 0 (runtime) is always invalid
    let mut checker = StageChecker::with_defaults();
    // Don't enter any function -- current_stage stays at 0

    let result = checker.check_quote(None, test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));
}

#[test]
fn test_stage_1_cannot_generate_stage_1() {
    // meta fn cannot generate meta code (should be stage 0)
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_quote(Some(1), test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));
}

// =============================================================================
// CROSS-STAGE CALL RESTRICTIONS
// =============================================================================

#[test]
fn test_same_stage_call_always_valid() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("a"), 1, test_span())
        .unwrap();
    checker
        .register_function(Text::from("b"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("a"), 1, test_span());

    assert!(checker.check_call(&Text::from("b"), 1, test_span()).is_ok());
}

#[test]
fn test_stage_0_same_stage_call() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime_a"), 0, test_span())
        .unwrap();
    checker
        .register_function(Text::from("runtime_b"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime_a"), 0, test_span());

    assert!(checker
        .check_call(&Text::from("runtime_b"), 0, test_span())
        .is_ok());
}

#[test]
fn test_higher_stage_cannot_call_lower() {
    // Stage 2 cannot directly call stage 1
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("stage2"), 2, test_span())
        .unwrap();
    checker
        .register_function(Text::from("stage1"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("stage2"), 2, test_span());

    let result = checker.check_call(&Text::from("stage1"), 1, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));

    if let Err(StageError::CrossStageCall {
        caller_stage,
        callee_stage,
        ..
    }) = result
    {
        assert_eq!(caller_stage, 2);
        assert_eq!(callee_stage, 1);
    }
}

#[test]
fn test_lower_stage_cannot_call_higher() {
    // Stage 0 cannot call stage 1 (not yet compiled)
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime"), 0, test_span())
        .unwrap();
    checker
        .register_function(Text::from("meta"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime"), 0, test_span());

    let result = checker.check_call(&Text::from("meta"), 1, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
}

#[test]
fn test_stage_1_cannot_call_stage_0_directly() {
    // Stage 1 cannot call stage 0 directly (must generate code via quote)
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker
        .register_function(Text::from("runtime_fn"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_call(&Text::from("runtime_fn"), 0, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
}

// =============================================================================
// STAGE 0 CANNOT REFERENCE STAGE 1+ VALUES
// =============================================================================

#[test]
fn test_runtime_cannot_reference_meta_value() {
    // Stage 0 code referencing a stage 1 variable is an error
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime_fn"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime_fn"), 0, test_span());

    let result = checker.check_variable_reference(&Text::from("meta_var"), 1, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
}

#[test]
fn test_runtime_cannot_reference_stage2_value() {
    // Stage 0 code referencing a stage 2 variable is an error
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime_fn"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime_fn"), 0, test_span());

    let result = checker.check_variable_reference(&Text::from("meta2_var"), 2, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
}

#[test]
fn test_runtime_can_reference_runtime_value() {
    // Stage 0 code referencing a stage 0 variable is fine
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime_fn"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime_fn"), 0, test_span());

    assert!(checker
        .check_variable_reference(&Text::from("runtime_var"), 0, test_span())
        .is_ok());
}

#[test]
fn test_meta_can_reference_same_stage_value() {
    // Stage 1 code referencing a stage 1 variable is fine
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    assert!(checker
        .check_variable_reference(&Text::from("meta_var"), 1, test_span())
        .is_ok());
}

#[test]
fn test_meta_can_reference_lower_stage_value() {
    // Stage 1 code referencing a stage 0 variable is fine
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    assert!(checker
        .check_variable_reference(&Text::from("runtime_var"), 0, test_span())
        .is_ok());
}

#[test]
fn test_meta_cannot_reference_higher_stage_value() {
    // Stage 1 code referencing a stage 2 variable is an error
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_variable_reference(&Text::from("meta2_var"), 2, test_span());
    assert!(matches!(result, Err(StageError::CrossStageCall { .. })));
}

// =============================================================================
// QUOTE TYPE CORRECTNESS
// =============================================================================

#[test]
fn test_quote_type_at_correct_stage() {
    // Types used in generated code must be at or below the target stage
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    // Stage 0 type used in stage 0 generated code -- valid
    assert!(checker
        .check_quote_type(0, &Text::from("Int"), 0, test_span())
        .is_ok());
}

#[test]
fn test_quote_type_stage_too_high() {
    // Stage 1 type cannot be used in stage 0 generated code
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_quote_type(0, &Text::from("MetaType"), 1, test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));
}

#[test]
fn test_quote_type_lower_stage_ok() {
    // Stage 0 type used in stage 1 generated code -- valid
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta2_fn"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta2_fn"), 2, test_span());

    assert!(checker
        .check_quote_type(1, &Text::from("Int"), 0, test_span())
        .is_ok());
}

// =============================================================================
// SPLICE/UNQUOTE VALIDATION
// =============================================================================

#[test]
fn test_splice_at_current_stage() {
    // Splice expression must be at the current stage
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    assert!(checker.check_splice(1, test_span()).is_ok());
}

#[test]
fn test_splice_wrong_stage() {
    // Splice from wrong stage is an error
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_splice(0, test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));
}

#[test]
fn test_splice_higher_stage() {
    // Splice from higher stage is also an error
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta_fn"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta_fn"), 1, test_span());

    let result = checker.check_splice(2, test_span());
    assert!(matches!(result, Err(StageError::StageMismatch { .. })));
}

// =============================================================================
// STAGE OVERFLOW
// =============================================================================

#[test]
fn test_stage_overflow_default_max() {
    // Default max_stage is 2, so stage 3 is overflow
    let mut checker = StageChecker::with_defaults();
    let result = checker.register_function(Text::from("too_high"), 3, test_span());
    assert!(matches!(result, Err(StageError::StageOverflow { .. })));

    if let Err(StageError::StageOverflow {
        used_stage,
        max_stage,
        ..
    }) = result
    {
        assert_eq!(used_stage, 3);
        assert_eq!(max_stage, 2);
    }
}

#[test]
fn test_stage_overflow_custom_max() {
    // Custom max_stage = 5
    let config = StageConfig {
        max_stage: 5,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);

    // Stage 5 is OK
    assert!(checker
        .register_function(Text::from("stage5"), 5, test_span())
        .is_ok());

    // Stage 6 is overflow
    let result = checker.register_function(Text::from("stage6"), 6, test_span());
    assert!(matches!(result, Err(StageError::StageOverflow { .. })));
}

#[test]
fn test_stage_0_and_1_always_valid() {
    let mut checker = StageChecker::with_defaults();
    assert!(checker
        .register_function(Text::from("rt"), 0, test_span())
        .is_ok());
    assert!(checker
        .register_function(Text::from("meta"), 1, test_span())
        .is_ok());
    assert!(checker
        .register_function(Text::from("meta2"), 2, test_span())
        .is_ok());
}

// =============================================================================
// CYCLIC STAGE DEPENDENCIES
// =============================================================================

#[test]
fn test_cyclic_stage_detection() {
    // Cycle detection works through the call_path managed by enter/exit_function.
    // A cross-stage call from B back to A triggers the cycle check.
    // Use non-strict mode so cross-stage calls record rather than error,
    // but the cycle detection still triggers.
    let config = StageConfig {
        strict_cross_stage: false,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);
    checker
        .register_function(Text::from("A"), 2, test_span())
        .unwrap();
    checker
        .register_function(Text::from("B"), 1, test_span())
        .unwrap();

    // Enter A (stage 2), which pushes "A" onto the call_path
    checker.enter_function(&Text::from("A"), 2, test_span());
    // Enter B (stage 1), which pushes "B" onto the call_path
    checker.enter_function(&Text::from("B"), 1, test_span());

    // B tries to call A (cross-stage) -- "A" is on the call_path -> cycle
    let result = checker.check_call(&Text::from("A"), 2, test_span());
    assert!(matches!(result, Err(StageError::CyclicStage { .. })));

    if let Err(StageError::CyclicStage { cycle, start, .. }) = result {
        assert_eq!(start, Text::from("A"));
        assert!(cycle.len() >= 2);
    }
}

// =============================================================================
// STAGE ESCAPE
// =============================================================================

#[test]
fn test_escape_to_lower_stage_valid() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta2"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta2"), 2, test_span());

    // From stage 2, can escape to stage 0 or 1
    assert!(checker.check_stage_escape(0, test_span()).is_ok());
    assert!(checker.check_stage_escape(1, test_span()).is_ok());
}

#[test]
fn test_escape_to_same_stage_invalid() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta2"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta2"), 2, test_span());

    let result = checker.check_stage_escape(2, test_span());
    assert!(matches!(result, Err(StageError::InvalidStageEscape { .. })));
}

#[test]
fn test_escape_to_higher_stage_invalid() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta1"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta1"), 1, test_span());

    let result = checker.check_stage_escape(1, test_span());
    assert!(matches!(result, Err(StageError::InvalidStageEscape { .. })));

    let result = checker.check_stage_escape(2, test_span());
    assert!(matches!(result, Err(StageError::InvalidStageEscape { .. })));
}

// =============================================================================
// WARNINGS
// =============================================================================

#[test]
fn test_unused_stage_warning() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("unused_meta"), 1, test_span())
        .unwrap();

    // Don't invoke it
    let warnings = checker.collect_warnings();
    assert!(warnings
        .iter()
        .any(|w| matches!(w, StageWarning::UnusedStage { function_name, .. } if function_name.as_str() == "unused_meta")));
}

#[test]
fn test_invoked_no_unused_warning() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("used_meta"), 1, test_span())
        .unwrap();

    // Mark as invoked
    checker.mark_invoked(&Text::from("used_meta"));

    let warnings = checker.collect_warnings();
    assert!(!warnings.iter().any(|w| matches!(
        w,
        StageWarning::UnusedStage { function_name, .. } if function_name.as_str() == "used_meta"
    )));
}

#[test]
fn test_stage_0_functions_no_unused_warning() {
    // Stage 0 (runtime) functions should never get unused stage warnings
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime"), 0, test_span())
        .unwrap();

    let warnings = checker.collect_warnings();
    assert!(!warnings
        .iter()
        .any(|w| matches!(w, StageWarning::UnusedStage { .. })));
}

#[test]
fn test_stage_downgrade_warning() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("over_staged"), 2, test_span())
        .unwrap();

    // Enter and record that we only generate stage 0 code (not stage 1)
    checker.enter_function(&Text::from("over_staged"), 2, test_span());
    if let Some(info) = checker.get_function_info_mut(&Text::from("over_staged")) {
        info.record_generated_stage(0);
        info.mark_invoked();
    }
    checker.exit_function();

    let warnings = checker.collect_warnings();
    assert!(warnings.iter().any(|w| matches!(
        w,
        StageWarning::StageDowngrade {
            current_stage: 2,
            suggested_stage: 1,
            ..
        }
    )));
}

#[test]
fn test_no_downgrade_warning_when_correct() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("correct"), 2, test_span())
        .unwrap();

    // Generate stage 1 code (correct for stage 2)
    checker.enter_function(&Text::from("correct"), 2, test_span());
    if let Some(info) = checker.get_function_info_mut(&Text::from("correct")) {
        info.record_generated_stage(1);
        info.mark_invoked();
    }
    checker.exit_function();

    let warnings = checker.collect_warnings();
    assert!(!warnings
        .iter()
        .any(|w| matches!(w, StageWarning::StageDowngrade { .. })));
}

// =============================================================================
// NESTED FUNCTION CONTEXTS
// =============================================================================

#[test]
fn test_nested_function_restore() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("outer"), 2, test_span())
        .unwrap();
    checker
        .register_function(Text::from("inner"), 1, test_span())
        .unwrap();

    checker.enter_function(&Text::from("outer"), 2, test_span());
    assert_eq!(checker.current_stage(), 2);

    checker.enter_function(&Text::from("inner"), 1, test_span());
    assert_eq!(checker.current_stage(), 1);

    checker.exit_function();
    assert_eq!(checker.current_stage(), 2);

    checker.exit_function();
    assert_eq!(checker.current_stage(), 0);
}

#[test]
fn test_deeply_nested_contexts() {
    let config = StageConfig {
        max_stage: 5,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);

    for i in 0..=5 {
        checker
            .register_function(Text::from(format!("fn_{}", i)), i, test_span())
            .unwrap();
    }

    // Enter functions from stage 5 down to 0
    for i in (0..=5).rev() {
        checker.enter_function(&Text::from(format!("fn_{}", i)), i, test_span());
        assert_eq!(checker.current_stage(), i);
    }

    // Exit all
    for i in 0..=5 {
        checker.exit_function();
        if i < 5 {
            assert_eq!(checker.current_stage(), i + 1);
        }
    }
    assert_eq!(checker.current_stage(), 0);
}

// =============================================================================
// CONFIGURATION
// =============================================================================

#[test]
fn test_non_strict_mode_allows_cross_stage() {
    let config = StageConfig {
        strict_cross_stage: false,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);
    checker
        .register_function(Text::from("meta2"), 2, test_span())
        .unwrap();
    checker
        .register_function(Text::from("meta1"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta2"), 2, test_span());

    // In non-strict mode, cross-stage calls are allowed (but recorded)
    assert!(checker
        .check_call(&Text::from("meta1"), 1, test_span())
        .is_ok());
}

#[test]
fn test_disable_unused_warnings() {
    let config = StageConfig {
        warn_unused_stage: false,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);
    checker
        .register_function(Text::from("unused"), 2, test_span())
        .unwrap();

    let warnings = checker.collect_warnings();
    assert!(!warnings
        .iter()
        .any(|w| matches!(w, StageWarning::UnusedStage { .. })));
}

#[test]
fn test_disable_downgrade_warnings() {
    let config = StageConfig {
        warn_stage_downgrade: false,
        ..Default::default()
    };
    let mut checker = StageChecker::new(config);
    checker
        .register_function(Text::from("over_staged"), 2, test_span())
        .unwrap();

    checker.enter_function(&Text::from("over_staged"), 2, test_span());
    if let Some(info) = checker.get_function_info_mut(&Text::from("over_staged")) {
        info.record_generated_stage(0);
        info.mark_invoked();
    }
    checker.exit_function();

    let warnings = checker.collect_warnings();
    assert!(!warnings
        .iter()
        .any(|w| matches!(w, StageWarning::StageDowngrade { .. })));
}

// =============================================================================
// CLEAR AND REUSE
// =============================================================================

#[test]
fn test_clear_resets_state() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("fn1"), 1, test_span())
        .unwrap();
    checker.enter_function(&Text::from("fn1"), 1, test_span());

    assert_eq!(checker.current_stage(), 1);

    checker.clear();

    assert_eq!(checker.current_stage(), 0);
    assert!(checker.get_function_info(&Text::from("fn1")).is_none());
}

// =============================================================================
// FUNCTION STAGE INFO
// =============================================================================

#[test]
fn test_function_stage_info_record_call() {
    let mut info = FunctionStageInfo::new(Text::from("test"), 2, test_span());
    info.record_call(Text::from("helper"), 2);
    assert_eq!(info.calls.len(), 1);
    assert_eq!(*info.calls.get(&Text::from("helper")).unwrap(), 2);
}

#[test]
fn test_function_stage_info_generated_stage() {
    let mut info = FunctionStageInfo::new(Text::from("test"), 2, test_span());
    info.record_generated_stage(1);
    assert_eq!(info.min_generated_stage, Some(1));
    assert_eq!(info.max_generated_stage, Some(1));

    info.record_generated_stage(0);
    assert_eq!(info.min_generated_stage, Some(0));
    assert_eq!(info.max_generated_stage, Some(1));
}

#[test]
fn test_function_stage_info_can_downgrade() {
    let mut info = FunctionStageInfo::new(Text::from("test"), 2, test_span());
    // No generated code yet -- can't determine downgrade
    assert!(info.can_downgrade().is_none());

    // Only generates stage 0 code (not stage 1) -- should suggest downgrade to 1
    info.record_generated_stage(0);
    assert_eq!(info.can_downgrade(), Some(1));
}

#[test]
fn test_function_stage_info_no_downgrade_when_correct() {
    let mut info = FunctionStageInfo::new(Text::from("test"), 2, test_span());
    info.record_generated_stage(1); // generates stage 1 (correct for stage 2)
    assert!(info.can_downgrade().is_none());
}

#[test]
fn test_function_stage_info_stage_1_no_downgrade() {
    // Stage 1 cannot be downgraded further
    let mut info = FunctionStageInfo::new(Text::from("test"), 1, test_span());
    info.record_generated_stage(0);
    assert!(info.can_downgrade().is_none());
}

#[test]
fn test_function_stage_info_stage_0_no_downgrade() {
    // Stage 0 cannot be downgraded
    let info = FunctionStageInfo::new(Text::from("test"), 0, test_span());
    assert!(info.can_downgrade().is_none());
}

// =============================================================================
// ERROR HINT QUALITY
// =============================================================================

#[test]
fn test_cross_stage_call_hint_downward() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("meta2"), 2, test_span())
        .unwrap();
    checker.enter_function(&Text::from("meta2"), 2, test_span());

    let result = checker.check_call(&Text::from("stage1_fn"), 1, test_span());
    if let Err(StageError::CrossStageCall { hint, .. }) = result {
        assert!(hint.as_str().contains("quote"));
    }
}

#[test]
fn test_cross_stage_call_hint_upward() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime"), 0, test_span());

    let result = checker.check_call(&Text::from("meta_fn"), 1, test_span());
    if let Err(StageError::CrossStageCall { hint, .. }) = result {
        assert!(hint.as_str().contains("not yet compiled"));
    }
}

#[test]
fn test_variable_reference_hint() {
    let mut checker = StageChecker::with_defaults();
    checker
        .register_function(Text::from("runtime"), 0, test_span())
        .unwrap();
    checker.enter_function(&Text::from("runtime"), 0, test_span());

    let result = checker.check_variable_reference(&Text::from("meta_val"), 1, test_span());
    if let Err(StageError::CrossStageCall { hint, .. }) = result {
        assert!(hint.as_str().contains("compile-time"));
    }
}
