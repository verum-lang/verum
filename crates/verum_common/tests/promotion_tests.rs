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
// Tests for promotion module
// Migrated from src/promotion.rs per CLAUDE.md standards

use verum_common::Maybe;
use verum_common::promotion::*;

#[test]
fn test_promotion_strategy_by_ref_count() {
    let strategy = PromotionStrategy::ByRefCount(100);
    let context = PromotionContext::from_ref_count(150);

    assert!(strategy.should_promote(&context));
}

#[test]
fn test_promotion_strategy_by_confidence() {
    let strategy = PromotionStrategy::ByConfidence(0.95);
    let context = PromotionContext::from_confidence(0.98);

    assert!(strategy.should_promote(&context));
}

#[test]
fn test_promotion_strategy_by_analysis() {
    let strategy = PromotionStrategy::ByAnalysis(EscapeAnalysisResult::DoesNotEscape);
    let context = PromotionContext::from_analysis(EscapeAnalysisResult::DoesNotEscape, 1.0);

    assert!(strategy.should_promote(&context));
}

#[test]
fn test_promotion_policy() {
    let policy = StandardPromotionPolicy::by_ref_count(100);
    let context = PromotionContext::from_ref_count(150);

    assert!(policy.should_promote(&context));
}

#[test]
fn test_composite_policy_any() {
    let policy = CompositePromotionPolicy::new(vec![
        PromotionStrategy::ByRefCount(200),    // Would fail
        PromotionStrategy::ByConfidence(0.95), // Would succeed
    ]);

    let context = PromotionContext::from_ref_count(150).with_confidence(0.98);

    assert!(policy.should_promote(&context)); // ANY strategy succeeds
}

#[test]
fn test_composite_policy_all() {
    let policy = CompositePromotionPolicy::require_all(vec![
        PromotionStrategy::ByRefCount(100),    // Would succeed
        PromotionStrategy::ByConfidence(0.95), // Would succeed
    ]);

    let context = PromotionContext::from_ref_count(150).with_confidence(0.98);

    assert!(policy.should_promote(&context)); // ALL strategies succeed
}

#[test]
fn test_promotion_decision() {
    let policy = StandardPromotionPolicy::by_ref_count(100);
    let context = PromotionContext::from_ref_count(150).with_ref_id(RefId(42));

    let decision = PromotionDecision::from_policy(&policy, &context);

    assert!(decision.should_promote);
    assert_eq!(decision.estimated_gain_ns, 15);
}

#[test]
fn test_promotion_statistics() {
    let mut stats = PromotionStatistics::new();

    let decision1 = PromotionDecision::new(
        Maybe::Some(RefId(1)),
        true,
        PromotionStrategy::ByRefCount(100),
        0.98,
        15,
    );

    let decision2 = PromotionDecision::new(
        Maybe::Some(RefId(2)),
        false,
        PromotionStrategy::ByRefCount(100),
        0.80,
        0,
    );

    stats.record_decision(&decision1);
    stats.record_decision(&decision2);

    assert_eq!(stats.total_decisions, 2);
    assert_eq!(stats.promotions, 1);
    assert_eq!(stats.kept_at_tier, 1);
    assert_eq!(stats.promotion_rate(), 0.5);
}

#[test]
fn test_reference_tier_promotion() {
    let managed = ReferenceTier::Managed;
    let checked = ReferenceTier::Checked;
    let unsafe_ref = ReferenceTier::Unsafe;

    // Valid promotions
    assert!(managed.can_promote_to(checked));
    assert!(managed.can_promote_to(unsafe_ref));
    assert!(checked.can_promote_to(unsafe_ref));

    // Invalid promotions
    assert!(!checked.can_promote_to(managed));
    assert!(!unsafe_ref.can_promote_to(managed));
    assert!(!unsafe_ref.can_promote_to(checked));
}

#[test]
fn test_reference_tier_degradation() {
    let managed = ReferenceTier::Managed;
    let checked = ReferenceTier::Checked;
    let unsafe_ref = ReferenceTier::Unsafe;

    // Valid degradations
    assert!(checked.can_degrade_to(managed));
    assert!(unsafe_ref.can_degrade_to(checked));
    assert!(unsafe_ref.can_degrade_to(managed));

    // Invalid degradations
    assert!(!managed.can_degrade_to(checked));
    assert!(!managed.can_degrade_to(unsafe_ref));
    assert!(!checked.can_degrade_to(unsafe_ref));
}

#[test]
fn test_escape_analysis_can_promote() {
    assert!(EscapeAnalysisResult::DoesNotEscape.can_promote());
    assert!(!EscapeAnalysisResult::EscapesViaReturn.can_promote());
    assert!(!EscapeAnalysisResult::EscapesViaHeap.can_promote());
}
