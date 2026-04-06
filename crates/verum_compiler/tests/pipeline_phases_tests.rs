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
//
// Tests each phase independently and the integrated pipeline.
// All types are now implemented in verum_compiler.

use std::path::PathBuf;
use std::time::Duration;
use verum_compiler::phases::PhaseMetrics;
use verum_compiler::{
    DiagnosticsEngine, ExecutionTier, Feature, GracefulFallback, IncrementalCompiler, Profile,
};

#[test]
fn test_incremental_compilation() {
    // Test caching and invalidation
    let mut compiler = IncrementalCompiler::new();
    assert!(compiler.needs_recompile(&PathBuf::from("test.vr")));
    let stats = compiler.stats();
    assert_eq!(stats.cached_modules, 0);
}

#[test]
fn test_diagnostics_engine() {
    // Test diagnostic emission
    let engine = DiagnosticsEngine::new();
    assert_eq!(engine.error_count(), 0);
    assert_eq!(engine.warning_count(), 0);
    assert!(!engine.has_errors());
}

#[test]
fn test_profile_system() {
    // Test profile features
    let app_profile = Profile::Application;
    assert!(app_profile.is_feature_enabled(Feature::RefinementTypes));
    assert!(!app_profile.is_feature_enabled(Feature::UnsafeCode));
    let systems_profile = Profile::Systems;
    assert!(systems_profile.is_feature_enabled(Feature::UnsafeCode));
}

#[test]
fn test_graceful_fallback() {
    // Test tier fallback: when AOT fails, should fall back to Interpreter
    let mut fallback = GracefulFallback::new(ExecutionTier::Aot);
    assert_eq!(fallback.active_tier(), ExecutionTier::Aot);
    let tier = fallback.fallback("AOT compilation unavailable");
    assert_eq!(tier, ExecutionTier::Interpreter);
}

#[test]
fn test_performance_metrics() {
    // Test metrics collection
    let metrics = PhaseMetrics::new("Test Phase")
        .with_duration(Duration::from_millis(100))
        .with_items_processed(42);
    assert_eq!(metrics.phase_name, "Test Phase");
    assert_eq!(metrics.items_processed, 42);
}
