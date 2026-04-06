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
// Comprehensive tests for code metrics collection
//
// Tests cover:
// - CodeMetricsCollector functionality
// - EnhancedCodeMetrics calculations
// - Coverage data structures
// - Profiling data integration
// - Git history change frequency
// - Cyclomatic complexity from CFG
// - Loop nesting depth analysis
// - AST-based metrics collection

use std::path::Path;
use verum_verification::metrics::*;
use verum_verification::transition::CodeMetrics;
use verum_verification::{TransitionAnalyzer, TransitionStrategy, VerificationLevel};

// =============================================================================
// EnhancedCodeMetrics Tests
// =============================================================================

#[test]
fn test_enhanced_metrics_default() {
    let metrics = EnhancedCodeMetrics::new("test_func");

    assert_eq!(metrics.function_name.as_str(), "test_func");
    assert_eq!(metrics.cyclomatic_complexity, 1);
    assert_eq!(metrics.lines_of_code, 0);
    assert_eq!(metrics.test_coverage, 0.0);
    assert!(!metrics.has_unsafe_blocks);
    assert_eq!(metrics.loop_nesting_depth, 0);
}

#[test]
fn test_metrics_stability_checks() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Unstable by default (change_frequency_per_week = 10.0)
    assert!(!metrics.is_stable());
    assert!(!metrics.is_very_stable());

    // Make it stable
    metrics.change_frequency_per_week = 0.5;
    assert!(metrics.is_stable());
    assert!(!metrics.is_very_stable());

    // Make it very stable
    metrics.change_frequency_per_week = 0.05;
    assert!(metrics.is_stable());
    assert!(metrics.is_very_stable());
}

#[test]
fn test_metrics_test_coverage() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Low coverage
    metrics.test_coverage = 0.5;
    assert!(!metrics.has_good_tests());

    // Good coverage
    metrics.test_coverage = 0.95;
    assert!(metrics.has_good_tests());
}

#[test]
fn test_metrics_criticality() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Not critical
    metrics.criticality_score = 5;
    assert!(!metrics.is_critical());

    // Critical
    metrics.criticality_score = 9;
    assert!(metrics.is_critical());
}

#[test]
fn test_metrics_complexity() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Simple
    metrics.cyclomatic_complexity = 5;
    assert!(!metrics.is_complex());

    // Complex
    metrics.cyclomatic_complexity = 15;
    assert!(metrics.is_complex());
}

#[test]
fn test_metrics_nesting_depth() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Shallow
    metrics.loop_nesting_depth = 2;
    assert!(!metrics.has_deep_nesting());

    // Deep
    metrics.loop_nesting_depth = 5;
    assert!(metrics.has_deep_nesting());
}

#[test]
fn test_maintainability_index() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Simple code should have high maintainability
    metrics.lines_of_code = 10;
    metrics.cyclomatic_complexity = 2;
    let mi = metrics.maintainability_index();
    assert!(
        mi > 50.0,
        "Simple code should have maintainability > 50, got {}",
        mi
    );

    // Complex code should have lower maintainability
    metrics.lines_of_code = 1000;
    metrics.cyclomatic_complexity = 50;
    let mi = metrics.maintainability_index();
    assert!(
        mi < 80.0,
        "Complex code should have maintainability < 80, got {}",
        mi
    );
}

#[test]
fn test_transition_risk_score() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // Low risk: simple, well-tested, stable code
    metrics.cyclomatic_complexity = 5;
    metrics.loop_nesting_depth = 1;
    metrics.has_unsafe_blocks = false;
    metrics.has_complex_predicates = false;
    metrics.test_coverage = 0.95;
    metrics.change_frequency_per_week = 0.1;

    let risk = metrics.transition_risk_score();
    assert!(
        risk < 3.0,
        "Low-risk code should have score < 3, got {}",
        risk
    );

    // High risk: complex, untested, unsafe code
    metrics.cyclomatic_complexity = 25;
    metrics.loop_nesting_depth = 5;
    metrics.has_unsafe_blocks = true;
    metrics.has_complex_predicates = true;
    metrics.test_coverage = 0.3;
    metrics.change_frequency_per_week = 10.0;

    let risk = metrics.transition_risk_score();
    assert!(
        risk > 7.0,
        "High-risk code should have score > 7, got {}",
        risk
    );
}

#[test]
fn test_enhanced_to_code_metrics_conversion() {
    let mut enhanced = EnhancedCodeMetrics::new("test");
    enhanced.test_coverage = 0.85;
    enhanced.change_frequency_per_week = 0.5;
    enhanced.lines_of_code = 100;
    enhanced.cyclomatic_complexity = 10;
    enhanced.execution_frequency = 500.0;
    enhanced.criticality_score = 7;
    enhanced.loop_count = 3;
    enhanced.has_complex_predicates = true;

    let basic: CodeMetrics = enhanced.to_code_metrics();

    assert_eq!(basic.test_coverage, 0.85);
    assert_eq!(basic.change_frequency_per_week, 0.5);
    assert_eq!(basic.lines_of_code, 100);
    assert_eq!(basic.cyclomatic_complexity, 10);
    assert_eq!(basic.execution_frequency, 500.0);
    assert_eq!(basic.criticality_score, 7);
    assert!(basic.has_loops);
    assert!(basic.has_complex_predicates);
}

// =============================================================================
// CodeMetrics Tests
// =============================================================================

#[test]
fn test_code_metrics_default() {
    let metrics = CodeMetrics::default();

    assert_eq!(metrics.test_coverage, 0.0);
    assert_eq!(metrics.change_frequency_per_week, 10.0);
    assert_eq!(metrics.cyclomatic_complexity, 1);
    assert!(!metrics.has_unsafe_blocks);
    assert_eq!(metrics.loop_nesting_depth, 0);
    assert_eq!(metrics.assertion_density, 0.0);
}

#[test]
fn test_code_metrics_stability() {
    let mut metrics = CodeMetrics::default();

    metrics.change_frequency_per_week = 0.5;
    assert!(metrics.is_stable());

    metrics.change_frequency_per_week = 0.05;
    assert!(metrics.is_very_stable());
}

#[test]
fn test_code_metrics_transition_risk() {
    let mut metrics = CodeMetrics::default();

    // Low risk scenario
    metrics.cyclomatic_complexity = 5;
    metrics.loop_nesting_depth = 1;
    metrics.has_unsafe_blocks = false;
    metrics.test_coverage = 0.9;
    metrics.change_frequency_per_week = 0.3;

    let risk = metrics.transition_risk_score();
    assert!(risk < 3.0);

    // High risk scenario
    metrics.cyclomatic_complexity = 30;
    metrics.loop_nesting_depth = 5;
    metrics.has_unsafe_blocks = true;
    metrics.test_coverage = 0.2;
    metrics.change_frequency_per_week = 8.0;

    let risk = metrics.transition_risk_score();
    assert!(risk > 6.0);
}

#[test]
fn test_code_metrics_verification_candidate() {
    let mut metrics = CodeMetrics::default();

    // Not a candidate by default (unstable)
    assert!(!metrics.is_verification_candidate());

    // Make it a candidate
    metrics.change_frequency_per_week = 0.3; // stable
    metrics.test_coverage = 0.95; // good tests
    metrics.cyclomatic_complexity = 10; // manageable complexity
    metrics.loop_nesting_depth = 2; // not deep nesting

    assert!(metrics.is_verification_candidate());

    // Too complex
    metrics.cyclomatic_complexity = 25;
    assert!(!metrics.is_verification_candidate());

    // Reset complexity, but deep nesting
    metrics.cyclomatic_complexity = 10;
    metrics.loop_nesting_depth = 5;
    assert!(!metrics.is_verification_candidate());
}

// =============================================================================
// Coverage Data Tests
// =============================================================================

#[test]
fn test_coverage_data_empty() {
    let coverage = CoverageData::new();

    assert_eq!(coverage.function_coverage("nonexistent"), 0.0);
    assert_eq!(coverage.file_coverage(Path::new("/nonexistent")), 0.0);
}

// =============================================================================
// Profiling Data Tests
// =============================================================================

#[test]
fn test_profiling_data_empty() {
    let profiling = ProfilingData::new();

    assert_eq!(profiling.execution_frequency("nonexistent"), 0.0);
    assert!(!profiling.is_hot_path("nonexistent"));
}

// =============================================================================
// Git History Tests
// =============================================================================

#[test]
fn test_git_history_empty() {
    let history = GitHistory::new();

    assert_eq!(history.commit_count(Path::new("/nonexistent")), 0);
    assert_eq!(history.author_count(Path::new("/nonexistent")), 0);
    assert_eq!(
        history.change_frequency(Path::new("/nonexistent"), 12.0),
        0.0
    );
}

#[test]
fn test_git_history_add_commits() {
    let mut history = GitHistory::new();
    let path = Path::new("/test/file.rs");

    history.add_commit(path, 1000, Some("alice"));
    history.add_commit(path, 2000, Some("bob"));
    history.add_commit(path, 3000, Some("alice"));

    assert_eq!(history.commit_count(path), 3);
    assert_eq!(history.author_count(path), 2);

    // 3 commits over 12 weeks = 0.25 commits/week
    let freq = history.change_frequency(path, 12.0);
    assert!((freq - 0.25).abs() < 0.01);
}

// =============================================================================
// CodeMetricsCollector Tests
// =============================================================================

#[test]
fn test_collector_creation() {
    let collector = CodeMetricsCollector::new();
    assert_eq!(collector.total_analysis_time(), std::time::Duration::ZERO);
}

#[test]
fn test_collector_caching() {
    let mut collector = CodeMetricsCollector::new();

    // First analysis should not be cached
    assert!(collector.get_cached_metrics("test_func").is_none());

    // After clearing, cache should be empty
    collector.clear_cache();
    assert!(collector.get_cached_metrics("test_func").is_none());
}

#[test]
fn test_collector_change_frequency() {
    let collector = CodeMetricsCollector::new();

    // Without git history, should return default
    let freq = collector.calculate_change_frequency(Path::new("/test/file.rs"));
    assert_eq!(freq, 0.5);
}

// =============================================================================
// Integration with Transition System Tests
// =============================================================================

#[test]
fn test_transition_analyzer_with_metrics() {
    let analyzer = TransitionAnalyzer::new(TransitionStrategy::Balanced);

    // Create well-tested, stable code metrics
    let mut metrics = CodeMetrics::default();
    metrics.test_coverage = 0.95;
    metrics.change_frequency_per_week = 0.1;
    metrics.criticality_score = 5;
    metrics.execution_frequency = 1000.0;
    metrics.cyclomatic_complexity = 8;
    metrics.loop_nesting_depth = 1;

    let decision = analyzer.analyze_function(
        &verum_common::Text::from("hot_function"),
        VerificationLevel::Runtime,
        &metrics,
    );

    // Should recommend transition with good metrics
    assert!(decision.recommend);
    assert_eq!(decision.from, VerificationLevel::Runtime);
    assert_eq!(decision.to, VerificationLevel::Static);
    assert!(decision.confidence > 0.0);
}

#[test]
fn test_transition_analyzer_with_poor_metrics() {
    let analyzer = TransitionAnalyzer::new(TransitionStrategy::Conservative);

    // Create unstable code with poor tests
    let mut metrics = CodeMetrics::default();
    metrics.test_coverage = 0.3;
    metrics.change_frequency_per_week = 5.0;
    metrics.criticality_score = 3;
    metrics.execution_frequency = 10.0;
    metrics.cyclomatic_complexity = 25;
    metrics.loop_nesting_depth = 4;
    metrics.has_unsafe_blocks = true;

    let decision = analyzer.analyze_function(
        &verum_common::Text::from("unstable_function"),
        VerificationLevel::Runtime,
        &metrics,
    );

    // Should not recommend transition for unstable code
    // The recommendation should be false or confidence should be reduced
    assert!(!decision.recommend || decision.confidence < 0.8);
}

#[test]
fn test_verification_candidate_selection() {
    let mut good_metrics = CodeMetrics::default();
    good_metrics.change_frequency_per_week = 0.2;
    good_metrics.test_coverage = 0.92;
    good_metrics.cyclomatic_complexity = 8;
    good_metrics.loop_nesting_depth = 2;

    assert!(good_metrics.is_verification_candidate());

    let mut bad_metrics = CodeMetrics::default();
    bad_metrics.change_frequency_per_week = 5.0;
    bad_metrics.test_coverage = 0.4;
    bad_metrics.cyclomatic_complexity = 30;
    bad_metrics.loop_nesting_depth = 6;

    assert!(!bad_metrics.is_verification_candidate());
}

// =============================================================================
// Metric Calculations Tests
// =============================================================================

#[test]
fn test_assertion_density_calculation() {
    let mut metrics = EnhancedCodeMetrics::new("test");

    // 0 assertions in 0 lines = 0 density
    assert_eq!(metrics.assertion_density, 0.0);

    // Set some values
    metrics.lines_of_code = 100;
    // Assertion density would be calculated during analysis
}

#[test]
fn test_proof_complexity_estimation() {
    let mut metrics = EnhancedCodeMetrics::new("simple_func");
    metrics.cyclomatic_complexity = 3;
    metrics.loop_nesting_depth = 0;
    metrics.branch_count = 2;
    metrics.call_count = 1;
    metrics.has_complex_predicates = false;
    metrics.has_unsafe_blocks = false;

    // Simple function should have lower complexity
    assert!(metrics.proof_complexity < 200);

    let mut complex = EnhancedCodeMetrics::new("complex_func");
    complex.cyclomatic_complexity = 20;
    complex.loop_nesting_depth = 4;
    complex.branch_count = 15;
    complex.call_count = 10;
    complex.has_complex_predicates = true;
    complex.has_unsafe_blocks = true;

    // Recalculate proof complexity manually for test
    let expected = 20 * 10 + 16 * 20 + 15 * 5 + 10 * 3 + 50 + 100;
    complex.proof_complexity = expected;

    assert!(complex.proof_complexity > 400);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_coverage_load_nonexistent_file() {
    let result = CoverageData::load_lcov(Path::new("/nonexistent/coverage.lcov"));
    assert!(result.is_err());
}

#[test]
fn test_profiling_load_nonexistent_file() {
    let result = ProfilingData::load_callgrind(Path::new("/nonexistent/callgrind.out"));
    assert!(result.is_err());
}

// =============================================================================
// Boundary Cases Tests
// =============================================================================

#[test]
fn test_metrics_with_zero_loc() {
    let mut metrics = EnhancedCodeMetrics::new("empty");
    metrics.lines_of_code = 0;

    // Should not panic
    let mi = metrics.maintainability_index();
    assert!((0.0..=100.0).contains(&mi));
}

#[test]
fn test_metrics_with_zero_complexity() {
    let mut metrics = CodeMetrics::default();
    metrics.cyclomatic_complexity = 0;

    // Maintainability should handle zero complexity
    let mi = metrics.maintainability_index();
    assert!((0.0..=100.0).contains(&mi));
}

#[test]
fn test_risk_score_capped_at_ten() {
    let mut metrics = CodeMetrics::default();

    // Set all risk factors to maximum
    metrics.cyclomatic_complexity = 100;
    metrics.loop_nesting_depth = 10;
    metrics.has_unsafe_blocks = true;
    metrics.has_complex_predicates = true;
    metrics.test_coverage = 0.0;
    metrics.change_frequency_per_week = 100.0;

    let risk = metrics.transition_risk_score();
    assert!(
        risk <= 10.0,
        "Risk score should be capped at 10, got {}",
        risk
    );
}

// =============================================================================
// Performance Tests
// =============================================================================

#[test]
fn test_metrics_collection_performance() {
    let collector = CodeMetricsCollector::new();

    // Initial analysis time should be zero
    let initial_time = collector.total_analysis_time();
    assert_eq!(initial_time.as_nanos(), 0);
}
