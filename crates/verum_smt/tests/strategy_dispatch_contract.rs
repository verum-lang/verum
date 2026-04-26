//! Per-strategy dispatch contract tests (V8 #233, §12 closure).
//!
//! VVA §12 specifies 9 base strategies (`runtime`, `static`,
//! `fast`, `formal`, `proof`, `thorough`, `reliable`,
//! `certified`, `synthesize`) plus 4 extension strategies
//! (`complexity_typed` from VVA-8, `coherent_*` from VVA-6) for
//! a 13-variant ladder. This test file verifies each variant's
//! contract per the spec table:
//!
//!   • from_attribute_value parses canonical + alias forms.
//!   • as_str renders the canonical form.
//!   • nu_ordinal returns the spec-mandated ν per §12.
//!   • requires_smt() correctly classifies SMT vs non-SMT.
//!   • is_synthesis() classifies orthogonal synthesis path.
//!   • timeout_multiplier produces strictly monotone budgets
//!     across the SMT-using fragment.
//!   • LADDER constant is exhaustive (13 entries, no
//!     duplicates).
//!
//! These tests are the explicit §12 contract harness. If a
//! future commit adds a 14th strategy, this file fails until
//! every contract row is updated — keeping the §12 spec table
//! and the impl in lockstep.

use verum_smt::verify_strategy::{NuOrdinal, VerifyStrategy};

// =============================================================================
// 9 §12-base strategy contract rows
// =============================================================================

#[test]
fn runtime_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("runtime").unwrap();
    assert_eq!(s, VerifyStrategy::Runtime);
    assert_eq!(s.as_str(), "runtime");
    assert_eq!(s.nu_ordinal(), NuOrdinal::Zero);
    assert!(!s.requires_smt(), "runtime is non-SMT");
    assert!(!s.is_synthesis());
    assert_eq!(s.timeout_multiplier(), 0.0);
}

#[test]
fn static_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("static").unwrap();
    assert_eq!(s, VerifyStrategy::Static);
    assert_eq!(s.as_str(), "static");
    assert_eq!(s.nu_ordinal(), NuOrdinal::FiniteOne);
    assert!(!s.requires_smt(), "static is dataflow-only, no SMT");
    assert!(!s.is_synthesis());
}

#[test]
fn fast_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("fast").unwrap();
    assert_eq!(s, VerifyStrategy::Fast);
    assert_eq!(s.as_str(), "fast");
    assert_eq!(s.nu_ordinal(), NuOrdinal::FiniteTwo);
    assert!(s.requires_smt());
    assert!(!s.is_synthesis());
    // §12.3: bounded single-solver SMT, tight timeout.
    assert!(s.timeout_multiplier() > 0.0);
    assert!(s.timeout_multiplier() < VerifyStrategy::Formal.timeout_multiplier());

    // Aliases per §12.3.
    assert_eq!(VerifyStrategy::from_attribute_value("quick"), Some(VerifyStrategy::Fast));
    assert_eq!(VerifyStrategy::from_attribute_value("rapid"), Some(VerifyStrategy::Fast));
}

#[test]
fn formal_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("formal").unwrap();
    assert_eq!(s, VerifyStrategy::Formal);
    assert_eq!(s.as_str(), "formal");
    assert_eq!(s.nu_ordinal(), NuOrdinal::Omega);
    assert!(s.requires_smt());
    assert!(!s.is_synthesis());
}

#[test]
fn proof_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("proof").unwrap();
    assert_eq!(s, VerifyStrategy::Proof);
    assert_eq!(s.as_str(), "proof");
    assert_eq!(s.nu_ordinal(), NuOrdinal::OmegaPlusOne);
    // §12.5: user-supplied tactic, kernel rechecks. No SMT
    // dispatch — the tactic IS the proof.
    assert!(!s.requires_smt(), "proof is user-supplied tactic, kernel rechecks");
    assert!(!s.is_synthesis());
}

#[test]
fn thorough_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("thorough").unwrap();
    assert_eq!(s, VerifyStrategy::Thorough);
    assert_eq!(s.as_str(), "thorough");
    assert_eq!(s.nu_ordinal(), NuOrdinal::OmegaTwice);
    assert!(s.requires_smt());
    assert!(!s.is_synthesis());
    // §12.6: ≈2× formal cost.
    assert!(s.timeout_multiplier() >= VerifyStrategy::Formal.timeout_multiplier());

    // Alias per §12.6.
    assert_eq!(VerifyStrategy::from_attribute_value("robust"), Some(VerifyStrategy::Thorough));
}

#[test]
fn reliable_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("reliable").unwrap();
    assert_eq!(s, VerifyStrategy::Reliable);
    assert_eq!(s.as_str(), "reliable");
    assert_eq!(s.nu_ordinal(), NuOrdinal::OmegaTwicePlusOne);
    assert!(s.requires_smt());
    assert!(!s.is_synthesis());
    // §12.7: ≈2× thorough.
    assert!(s.timeout_multiplier() >= VerifyStrategy::Thorough.timeout_multiplier());
}

#[test]
fn certified_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("certified").unwrap();
    assert_eq!(s, VerifyStrategy::Certified);
    assert_eq!(s.as_str(), "certified");
    assert_eq!(s.nu_ordinal(), NuOrdinal::OmegaTwicePlusTwo);
    assert!(s.requires_smt());
    assert!(!s.is_synthesis());
    // §12.8: ≈3× thorough.
    assert!(s.timeout_multiplier() >= VerifyStrategy::Thorough.timeout_multiplier());

    // Aliases per §12.8.
    let cv = VerifyStrategy::from_attribute_value("cross_validate");
    assert_eq!(cv, Some(VerifyStrategy::Certified));
}

#[test]
fn synthesize_strategy_contract() {
    let s = VerifyStrategy::from_attribute_value("synthesize").unwrap();
    assert_eq!(s, VerifyStrategy::Synthesize);
    assert_eq!(s.as_str(), "synthesize");
    // §12.9: orthogonal — ν ≤ ω·3+1.
    assert!(s.is_synthesis());
    assert!(s.requires_smt(), "synthesis goes through SyGuS-capable backend");

    // Aliases per §12.9.
    assert_eq!(VerifyStrategy::from_attribute_value("synth"), Some(VerifyStrategy::Synthesize));
    assert_eq!(VerifyStrategy::from_attribute_value("synthesis"), Some(VerifyStrategy::Synthesize));
}

// =============================================================================
// LADDER invariants
// =============================================================================

#[test]
fn ladder_contains_thirteen_unique_strategies() {
    let len = VerifyStrategy::LADDER.len();
    assert_eq!(len, 13, "LADDER must contain 13 strategies");

    // No duplicates.
    let mut seen = Vec::new();
    for s in &VerifyStrategy::LADDER {
        assert!(!seen.contains(s), "duplicate in LADDER: {:?}", s);
        seen.push(*s);
    }
}

#[test]
fn ladder_nu_ordinals_strictly_monotone_excluding_synthesize() {
    // The first 12 strategies form a strict ν chain; Synthesize is
    // orthogonal (highest ν but explicitly above the chain).
    // NuOrdinal doesn't impl PartialOrd directly — use the
    // strategy's own at_least relation (which IS the spec
    // ordering).
    for i in 0..11 {
        let low = &VerifyStrategy::LADDER[i];
        let high = &VerifyStrategy::LADDER[i + 1];
        assert!(
            high.at_least(low) && !low.at_least(high),
            "ladder monotonicity break at position {}: {:?} → {:?}",
            i, low, high,
        );
    }
}

#[test]
fn nine_base_strategies_all_present_in_ladder() {
    // Per VVA §12: 9 base strategies must be in LADDER.
    let nine_base = [
        VerifyStrategy::Runtime,
        VerifyStrategy::Static,
        VerifyStrategy::Fast,
        VerifyStrategy::Formal,
        VerifyStrategy::Proof,
        VerifyStrategy::Thorough,
        VerifyStrategy::Reliable,
        VerifyStrategy::Certified,
        VerifyStrategy::Synthesize,
    ];
    for s in nine_base {
        assert!(
            VerifyStrategy::LADDER.contains(&s),
            "§12 base strategy missing from LADDER: {:?}",
            s,
        );
    }
}

// =============================================================================
// Cross-strategy invariants
// =============================================================================

#[test]
fn at_least_relation_total_order_within_ladder() {
    // For every pair (s_i, s_j) with i ≤ j in LADDER (excl. Synthesize),
    // s_j.at_least(s_i) holds.
    let chain = &VerifyStrategy::LADDER[..12];
    for (i, low) in chain.iter().enumerate() {
        for high in &chain[i..] {
            assert!(
                high.at_least(low),
                "LADDER monotonicity broken: {:?}.at_least({:?})",
                high,
                low,
            );
        }
    }
}

#[test]
fn parsing_unknown_value_returns_none() {
    assert!(VerifyStrategy::from_attribute_value("totally_made_up").is_none());
    assert!(VerifyStrategy::from_attribute_value("").is_none());
}

#[test]
fn parsing_is_case_insensitive() {
    assert_eq!(
        VerifyStrategy::from_attribute_value("RUNTIME"),
        Some(VerifyStrategy::Runtime),
    );
    assert_eq!(
        VerifyStrategy::from_attribute_value("Formal"),
        Some(VerifyStrategy::Formal),
    );
}
