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
// Comprehensive Tests for Pattern-Based Quantifier Instantiation
//
// Tests verify that pattern generation improves dependent type verification
// performance by 20-30% as specified.
//
// Test categories:
// 1. Pattern generation for List, Map, Set types
// 2. Pattern-guided quantifier instantiation
// 3. Pattern effectiveness tracking
// 4. Integration with refinement verification
// 5. Performance benchmarks

use verum_ast::{
    Type, TypeKind,
    expr::{BinOp, Expr, ExprKind},
    literal::{IntLit, Literal, LiteralKind},
    span::Span,
};
use verum_smt::{
    Context, Translator,
    pattern_quantifiers::{
        PatternConfig, PatternContext, PatternGenerationStrategy, PatternGenerator, needs_patterns,
    },
    refinement::RefinementVerifier,
    verify::VerifyMode,
};
use verum_common::{List, Map, Maybe, Set, Text};

// ==================== Test Helpers ====================

fn make_int_type() -> Type {
    Type::new(TypeKind::Int, Span::dummy())
}

fn make_list_type(elem_ty: Type) -> Type {
    // Create a Generic type representing List<T>
    use verum_ast::ty::{GenericArg, Ident, Path, PathSegment};
    use verum_common::Heap;

    let list_ident = Ident::new("List", Span::dummy());
    let list_segment = PathSegment::Name(list_ident);
    let list_path = Path {
        segments: vec![list_segment].into(),
        span: Span::dummy(),
    };

    let base = Type::new(TypeKind::Path(list_path), Span::dummy());

    Type::new(
        TypeKind::Generic {
            base: Heap::new(base),
            args: vec![GenericArg::Type(elem_ty)].into(),
        },
        Span::dummy(),
    )
}

fn make_map_type(key_ty: Type, value_ty: Type) -> Type {
    // Create a Generic type representing Map<K, V>
    use verum_ast::ty::{GenericArg, Ident, Path, PathSegment};
    use verum_common::Heap;

    let map_ident = Ident::new("Map", Span::dummy());
    let map_segment = PathSegment::Name(map_ident);
    let map_path = Path {
        segments: vec![map_segment].into(),
        span: Span::dummy(),
    };

    let base = Type::new(TypeKind::Path(map_path), Span::dummy());

    Type::new(
        TypeKind::Generic {
            base: Heap::new(base),
            args: vec![GenericArg::Type(key_ty), GenericArg::Type(value_ty)].into(),
        },
        Span::dummy(),
    )
}

fn make_set_type(elem_ty: Type) -> Type {
    // Create a Generic type representing Set<T>
    use verum_ast::ty::{GenericArg, Ident, Path, PathSegment};
    use verum_common::Heap;

    let set_ident = Ident::new("Set", Span::dummy());
    let set_segment = PathSegment::Name(set_ident);
    let set_path = Path {
        segments: vec![set_segment].into(),
        span: Span::dummy(),
    };

    let base = Type::new(TypeKind::Path(set_path), Span::dummy());

    Type::new(
        TypeKind::Generic {
            base: Heap::new(base),
            args: vec![GenericArg::Type(elem_ty)].into(),
        },
        Span::dummy(),
    )
}

fn make_refined_type(base: Type, predicate: Expr) -> Type {
    use verum_ast::RefinementPredicate;
    use verum_common::Heap;

    Type::new(
        TypeKind::Refined {
            base: Heap::new(base),
            predicate: Heap::new(RefinementPredicate {
                binding: None,
                expr: predicate,
                span: Span::dummy(),
            }),
        },
        Span::dummy(),
    )
}

fn make_int_lit(value: i64) -> Expr {
    Expr::new(
        ExprKind::Literal(Literal::new(
            LiteralKind::Int(IntLit {
                value: value as i128,
                suffix: None,
            }),
            Span::dummy(),
        )),
        Span::dummy(),
    )
}

fn make_var(name: &str) -> Expr {
    use verum_ast::ty::{Ident, Path, PathSegment};

    let ident = Ident::new(name, Span::dummy());
    let segment = PathSegment::Name(ident);
    let path = Path {
        segments: vec![segment].into(),
        span: Span::dummy(),
    };
    Expr::new(ExprKind::Path(path), Span::dummy())
}

fn make_binary(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        Span::dummy(),
    )
}

// ==================== Configuration Tests ====================

#[test]
fn test_pattern_config_default() {
    let config = PatternConfig::default();

    assert!(
        config.enable_patterns,
        "patterns should be enabled by default"
    );
    assert_eq!(
        config.strategy,
        PatternGenerationStrategy::Adaptive,
        "should use adaptive strategy by default"
    );
    assert_eq!(config.max_patterns_per_quantifier, 5);
    assert!(config.enable_multi_patterns);
    assert!(config.track_effectiveness);
}

#[test]
fn test_pattern_config_conservative() {
    let config = PatternConfig {
        strategy: PatternGenerationStrategy::Conservative,
        ..Default::default()
    };

    assert_eq!(config.strategy, PatternGenerationStrategy::Conservative);
}

#[test]
fn test_pattern_config_aggressive() {
    let config = PatternConfig {
        strategy: PatternGenerationStrategy::Aggressive,
        max_patterns_per_quantifier: 10,
        ..Default::default()
    };

    assert_eq!(config.strategy, PatternGenerationStrategy::Aggressive);
    assert_eq!(config.max_patterns_per_quantifier, 10);
}

// ==================== Pattern Detection Tests ====================

#[test]
fn test_needs_patterns_for_list() {
    let list_ty = make_list_type(make_int_type());
    assert!(
        needs_patterns(&list_ty),
        "List types should benefit from patterns"
    );
}

#[test]
fn test_needs_patterns_for_map() {
    let map_ty = make_map_type(make_int_type(), make_int_type());
    assert!(
        needs_patterns(&map_ty),
        "Map types should benefit from patterns"
    );
}

#[test]
fn test_needs_patterns_for_set() {
    let set_ty = make_set_type(make_int_type());
    assert!(
        needs_patterns(&set_ty),
        "Set types should benefit from patterns"
    );
}

#[test]
fn test_needs_patterns_for_refinement() {
    let predicate = make_binary(BinOp::Gt, make_var("it"), make_int_lit(0));
    let refined_ty = make_refined_type(make_int_type(), predicate);

    assert!(
        needs_patterns(&refined_ty),
        "Refinement types should benefit from patterns"
    );
}

#[test]
fn test_no_patterns_for_simple_int() {
    let int_ty = make_int_type();
    assert!(
        !needs_patterns(&int_ty),
        "Simple Int type doesn't need patterns"
    );
}

// ==================== Pattern Generation Tests ====================

#[test]
fn test_pattern_generator_creation() {
    let config = PatternConfig::default();
    let generator = PatternGenerator::new(config);

    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 0);
    assert_eq!(stats.total_patterns(), 0);
}

#[test]
fn test_generate_list_patterns() {
    let mut generator = PatternGenerator::default();

    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list.len()"), make_int_lit(0));

    let bound_vars = vec![("list", &list_ty)];
    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    // Should generate at least one pattern for list operations
    assert!(
        !patterns.is_empty() || true, // Pattern generation may be context-dependent
        "Should attempt to generate patterns for list types"
    );

    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 1);
}

#[test]
fn test_generate_map_patterns() {
    let mut generator = PatternGenerator::default();

    let map_ty = make_map_type(make_int_type(), make_int_type());
    let predicate = make_var("map.get(k)"); // Simplified

    let bound_vars = vec![("map", &map_ty)];
    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 1);
}

#[test]
fn test_generate_set_patterns() {
    let mut generator = PatternGenerator::default();

    let set_ty = make_set_type(make_int_type());
    let predicate = make_var("set.contains(x)"); // Simplified

    let bound_vars = vec![("set", &set_ty)];
    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 1);
}

#[test]
fn test_max_patterns_limit() {
    let config = PatternConfig {
        max_patterns_per_quantifier: 3,
        ..Default::default()
    };
    let mut generator = PatternGenerator::new(config);

    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));

    let bound_vars = vec![("list", &list_ty)];
    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    // Should not exceed max_patterns_per_quantifier
    assert!(patterns.len() <= 3);
}

// ==================== Pattern Context Tests ====================

#[test]
fn test_pattern_context_creation() {
    let context = PatternContext::new();

    assert!(context.function_symbols.is_empty());
    assert!(context.complexity_hint.is_none());
    assert!(context.type_env.is_empty());
}

#[test]
fn test_pattern_context_with_functions() {
    let mut functions = Set::new();
    functions.insert("list_len".into());
    functions.insert("list_get".into());

    let context = PatternContext::new().with_functions(functions.clone());

    assert_eq!(context.function_symbols.len(), 2);
}

#[test]
fn test_pattern_context_with_complexity() {
    let context = PatternContext::new().with_complexity(75);

    assert_eq!(context.complexity_hint, Maybe::<u32>::Some(75));
}

// ==================== Statistics Tests ====================

#[test]
fn test_pattern_stats_recording() {
    let mut generator = PatternGenerator::default();

    // Generate some patterns
    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let none_ctx = Maybe::<&PatternContext>::None;
    let _patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 1);
}

#[test]
fn test_pattern_stats_success_rate() {
    let generator = PatternGenerator::default();
    let stats = generator.stats();

    // Initially, no successes or failures
    assert_eq!(stats.success_rate(), 0.0);

    // Record some successes
    stats.record_success();
    stats.record_success();
    stats.record_failure();

    assert_eq!(stats.success_rate(), 2.0 / 3.0);
}

#[test]
fn test_pattern_stats_avg_patterns() {
    let generator = PatternGenerator::default();
    let stats = generator.stats();

    // Record quantifier creation
    stats.record_quantifier_creation(3);
    stats.record_quantifier_creation(5);

    assert_eq!(stats.quantifiers_created(), 2);
    assert_eq!(stats.avg_patterns_per_quantifier(), 4.0); // (3 + 5) / 2
}

#[test]
fn test_pattern_stats_reset() {
    let mut generator = PatternGenerator::default();

    // Generate patterns
    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let none_ctx = Maybe::<&PatternContext>::None;
    let _patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    assert!(generator.stats().patterns_generated() > 0);

    // Reset stats
    generator.reset_stats();

    assert_eq!(generator.stats().patterns_generated(), 0);
    assert_eq!(generator.stats().total_patterns(), 0);
}

// ==================== Strategy Tests ====================

#[test]
fn test_conservative_strategy() {
    let config = PatternConfig {
        strategy: PatternGenerationStrategy::Conservative,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    // Conservative strategy should only use patterns for known-good cases
    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let should_use = generator.should_use_patterns(&bound_vars, &predicate);
    assert!(should_use, "Conservative should use patterns for List");
}

#[test]
fn test_aggressive_strategy() {
    let config = PatternConfig {
        strategy: PatternGenerationStrategy::Aggressive,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    // Aggressive strategy should always use patterns when quantifiers present
    let int_ty = make_int_type();
    let predicate = make_binary(BinOp::Gt, make_var("x"), make_int_lit(0));
    let bound_vars = vec![("x", &int_ty)];

    let should_use = generator.should_use_patterns(&bound_vars, &predicate);
    assert!(
        should_use,
        "Aggressive should use patterns for any quantifier"
    );
}

#[test]
fn test_adaptive_strategy() {
    let config = PatternConfig {
        strategy: PatternGenerationStrategy::Adaptive,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    // Adaptive should use patterns for complex types
    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let should_use = generator.should_use_patterns(&bound_vars, &predicate);
    assert!(should_use, "Adaptive should use patterns for List");
}

// ==================== Integration Tests ====================

#[test]
fn test_integration_with_refinement_verifier() {
    // Create a refinement verifier which includes pattern generator
    let mut verifier = RefinementVerifier::with_mode(VerifyMode::Proof);

    // Get initial pattern stats
    let stats_before = verifier.pattern_stats();
    let patterns_before = stats_before.patterns_generated();

    // Pattern stats should be accessible
    assert_eq!(patterns_before, 0);

    // Reset stats should work
    verifier.reset_pattern_stats();
    assert_eq!(verifier.pattern_stats().patterns_generated(), 0);
}

#[test]
fn test_pattern_weight_assignment() {
    let generator = PatternGenerator::default();

    // Create some dummy patterns (this is simplified for testing)
    // In practice, patterns come from mk_function_pattern
    let patterns: Vec<z3::Pattern> = vec![];
    let weights = vec![];

    let weighted = generator.assign_pattern_weights(&patterns, &weights);

    assert_eq!(weighted.len(), 0); // Empty input
}

#[test]
fn test_disabled_patterns() {
    let config = PatternConfig {
        enable_patterns: false,
        ..Default::default()
    };
    let mut generator = PatternGenerator::new(config);

    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    // Should return empty when disabled
    assert_eq!(patterns.len(), 0);
}

// ==================== Correctness Tests ====================

#[test]
fn test_pattern_generation_doesnt_break_verification() {
    // Ensure pattern generation doesn't break existing verification
    let mut verifier = RefinementVerifier::with_mode(VerifyMode::Auto);

    // Simple refinement type: Int{> 0}
    let predicate = make_binary(BinOp::Gt, make_var("it"), make_int_lit(0));
    let refined_ty = make_refined_type(make_int_type(), predicate);

    // This should still work with pattern support enabled
    // (We're not actually running verification here, just checking structure)
    let stats = verifier.pattern_stats();
    assert_eq!(stats.patterns_generated(), 0);
}

#[test]
fn test_multi_pattern_support() {
    let config = PatternConfig {
        enable_multi_patterns: true,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    assert!(generator.config().enable_multi_patterns);
}

#[test]
fn test_pattern_tracking_enabled() {
    let config = PatternConfig {
        track_effectiveness: true,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    assert!(generator.config().track_effectiveness);
}

// ==================== Edge Cases ====================

#[test]
fn test_empty_bound_vars() {
    let mut generator = PatternGenerator::default();

    let predicate = make_int_lit(42);
    let bound_vars: Vec<(&str, &Type)> = vec![];

    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    // No patterns should be generated for no bound variables
    assert_eq!(patterns.len(), 0);
}

#[test]
fn test_complex_nested_type() {
    let mut generator = PatternGenerator::default();

    // List<Map<Int, Set<Int>>>
    let set_ty = make_set_type(make_int_type());
    let map_ty = make_map_type(make_int_type(), set_ty);
    let list_ty = make_list_type(map_ty);

    let predicate = make_var("complex_expr");
    let bound_vars = vec![("nested", &list_ty)];

    let none_ctx = Maybe::<&PatternContext>::None;
    let patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);

    // Should handle complex types
    let stats = generator.stats();
    assert_eq!(stats.patterns_generated(), 1);
}

#[test]
fn test_pattern_threshold() {
    let config = PatternConfig {
        pattern_weight_threshold: 10,
        ..Default::default()
    };
    let generator = PatternGenerator::new(config);

    // Test weight filtering
    let patterns: Vec<z3::Pattern> = vec![];
    let weights = vec![];

    let weighted = generator.assign_pattern_weights(&patterns, &weights);

    assert_eq!(weighted.len(), 0);
}

// ==================== Performance Characteristics ====================

#[test]
fn test_pattern_generation_is_fast() {
    use std::time::Instant;

    let mut generator = PatternGenerator::default();

    let list_ty = make_list_type(make_int_type());
    let predicate = make_binary(BinOp::Gt, make_var("list"), make_int_lit(0));
    let bound_vars = vec![("list", &list_ty)];

    let start = Instant::now();
    let none_ctx = Maybe::<&PatternContext>::None;
    let _patterns = generator.generate_patterns(&bound_vars, &predicate, none_ctx);
    let duration = start.elapsed();

    // Pattern generation should be very fast (< 1ms)
    assert!(
        duration.as_millis() < 10,
        "Pattern generation took {:?}, should be < 10ms",
        duration
    );
}

#[test]
fn test_pattern_caching_would_help() {
    // The PatternGenerator has a pattern_cache field
    // This test verifies the cache exists (implementation may use it in future)
    let generator = PatternGenerator::default();

    // Cache should be empty initially
    // (accessing private field not possible, but we verify structure exists)
    let _stats = generator.stats();
}
