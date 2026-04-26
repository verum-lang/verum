//! Nine-strategy `@verify(...)` ladder semantics (VUVA §2.3, §12).
//!
//! Verifies that every strategy listed in VUVA parses to a distinct
//! `VerifyStrategy` variant, is ranked in monotone-lift order, carries
//! the Diakrisis ν-invariant ordinal from the VUVA table, and dispatches
//! through the expected backend surface.

use verum_smt::verify_strategy::{NuOrdinal, VerifyStrategy};

// -----------------------------------------------------------------------------
// Parsing — every VUVA-named strategy must map to a distinct variant
// -----------------------------------------------------------------------------

#[test]
fn ladder_parses_all_nine_distinct_variants() {
    let values = [
        "runtime", "static", "fast", "formal", "proof",
        "thorough", "reliable", "certified", "synthesize",
    ];
    let parsed: Vec<_> = values
        .iter()
        .map(|v| VerifyStrategy::from_attribute_value(v).expect(v))
        .collect();

    // Each value parses to something …
    assert_eq!(parsed.len(), 9);
    // … and the nine are pairwise distinct.
    for i in 0..9 {
        for j in (i + 1)..9 {
            assert_ne!(
                parsed[i], parsed[j],
                "strategies {i} and {j} parsed to the same variant"
            );
        }
    }
}

#[test]
fn legacy_aliases_still_parse() {
    assert_eq!(VerifyStrategy::from_attribute_value("quick"), Some(VerifyStrategy::Fast));
    assert_eq!(VerifyStrategy::from_attribute_value("rapid"), Some(VerifyStrategy::Fast));
    assert_eq!(VerifyStrategy::from_attribute_value("robust"), Some(VerifyStrategy::Thorough));
    assert_eq!(VerifyStrategy::from_attribute_value("cross_validate"), Some(VerifyStrategy::Certified));
    assert_eq!(VerifyStrategy::from_attribute_value("synthesis"), Some(VerifyStrategy::Synthesize));
}

#[test]
fn proof_and_reliable_are_no_longer_aliases() {
    // Pre-VUVA these parsed as Formal / Thorough respectively.
    // Post-VUVA they are distinct variants.
    assert_eq!(
        VerifyStrategy::from_attribute_value("proof"),
        Some(VerifyStrategy::Proof)
    );
    assert_ne!(
        VerifyStrategy::from_attribute_value("proof"),
        Some(VerifyStrategy::Formal)
    );
    assert_eq!(
        VerifyStrategy::from_attribute_value("reliable"),
        Some(VerifyStrategy::Reliable)
    );
    assert_ne!(
        VerifyStrategy::from_attribute_value("reliable"),
        Some(VerifyStrategy::Thorough)
    );
}

#[test]
fn unknown_strategy_returns_none() {
    assert_eq!(VerifyStrategy::from_attribute_value(""), None);
    assert_eq!(VerifyStrategy::from_attribute_value("sound-and-complete"), None);
    assert_eq!(VerifyStrategy::from_attribute_value("auto"), None); // not in VUVA ladder
}

// -----------------------------------------------------------------------------
// Monotone-lift — strictly ascending rank per VUVA §2.3
// -----------------------------------------------------------------------------

#[test]
fn ladder_rank_is_strictly_ascending() {
    let ranks: Vec<u8> = VerifyStrategy::LADDER.iter().map(|s| s.rank()).collect();
    let sorted: Vec<u8> = {
        let mut r = ranks.clone();
        r.sort();
        r
    };
    assert_eq!(ranks, sorted, "LADDER is not monotonically increasing");
    // And distinct.
    for i in 1..ranks.len() {
        assert!(
            ranks[i - 1] < ranks[i],
            "rank({:?}) == rank({:?})",
            VerifyStrategy::LADDER[i - 1],
            VerifyStrategy::LADDER[i]
        );
    }
}

#[test]
fn at_least_enforces_monotone_lift() {
    // Certified ⇒ Reliable ⇒ Thorough ⇒ Proof ⇒ Formal ⇒ Fast ⇒ Static ⇒ Runtime.
    assert!(VerifyStrategy::Certified.at_least(&VerifyStrategy::Reliable));
    assert!(VerifyStrategy::Reliable.at_least(&VerifyStrategy::Thorough));
    assert!(VerifyStrategy::Thorough.at_least(&VerifyStrategy::Proof));
    assert!(VerifyStrategy::Proof.at_least(&VerifyStrategy::Formal));
    assert!(VerifyStrategy::Formal.at_least(&VerifyStrategy::Fast));
    assert!(VerifyStrategy::Fast.at_least(&VerifyStrategy::Static));
    assert!(VerifyStrategy::Static.at_least(&VerifyStrategy::Runtime));

    // Non-reflexive direction must be rejected.
    assert!(!VerifyStrategy::Runtime.at_least(&VerifyStrategy::Static));
    assert!(!VerifyStrategy::Formal.at_least(&VerifyStrategy::Proof));
    assert!(!VerifyStrategy::Thorough.at_least(&VerifyStrategy::Reliable));
}

// -----------------------------------------------------------------------------
// Diakrisis ν-invariant — VUVA §12 table
// -----------------------------------------------------------------------------

#[test]
fn nu_ordinals_match_vuva_table() {
    // VUVA §12 — strict-monotone ν-ladder: every strategy gets a
    // distinct ordinal so `0 < 1 < 2 < ω < ω+1 < ω·2 < ω·2+1 <
    // ω·2+2 < ω·3+1` holds in the formal sense.
    assert_eq!(VerifyStrategy::Runtime.nu_ordinal(), NuOrdinal::Zero);

    assert_eq!(VerifyStrategy::Static.nu_ordinal(), NuOrdinal::FiniteOne);
    assert_eq!(VerifyStrategy::Fast.nu_ordinal(),   NuOrdinal::FiniteTwo);

    assert_eq!(VerifyStrategy::Formal.nu_ordinal(), NuOrdinal::Omega);
    assert_eq!(VerifyStrategy::Proof.nu_ordinal(),  NuOrdinal::OmegaPlusOne);

    assert_eq!(VerifyStrategy::Thorough.nu_ordinal(),  NuOrdinal::OmegaTwice);
    assert_eq!(VerifyStrategy::Reliable.nu_ordinal(),  NuOrdinal::OmegaTwicePlusOne);
    assert_eq!(VerifyStrategy::Certified.nu_ordinal(), NuOrdinal::OmegaTwicePlusTwo);

    assert_eq!(VerifyStrategy::Synthesize.nu_ordinal(), NuOrdinal::OmegaThricePlusOne);
}

#[test]
fn nu_ordinal_rank_strictly_increases_with_strategy_rank() {
    // VUVA §2.3 strict-monotone claim: every step on the ladder
    // bumps the ν-rank by exactly one (no plateaus).
    let mut last_nu: Option<u8> = None;
    for s in VerifyStrategy::LADDER.iter() {
        let nu = s.nu_ordinal().rank();
        if let Some(prev) = last_nu {
            assert!(
                nu > prev,
                "ν-rank did not strictly increase at {s:?}: {nu} ≤ {prev}"
            );
        }
        last_nu = Some(nu);
    }
    // Final rank should be 12 (Synthesize) — thirteen distinct
    // strata mapped to ranks 0..=12 after the VFE-6 V1 + VFE-8 V0
    // ladder extension (added ComplexityTyped, CoherentStatic,
    // CoherentRuntime, Coherent).
    assert_eq!(last_nu, Some(12));
}

// -----------------------------------------------------------------------------
// Dispatch semantics — what backends / paths each strategy triggers
// -----------------------------------------------------------------------------

#[test]
fn proof_bypasses_smt_like_runtime_and_static() {
    assert!(!VerifyStrategy::Proof.requires_smt());
    assert!(!VerifyStrategy::Runtime.requires_smt());
    assert!(!VerifyStrategy::Static.requires_smt());
}

#[test]
fn fast_formal_thorough_reliable_certified_require_smt() {
    assert!(VerifyStrategy::Fast.requires_smt());
    assert!(VerifyStrategy::Formal.requires_smt());
    assert!(VerifyStrategy::Thorough.requires_smt());
    assert!(VerifyStrategy::Reliable.requires_smt());
    assert!(VerifyStrategy::Certified.requires_smt());
}

#[test]
fn reliable_and_certified_both_cross_validate() {
    assert!(VerifyStrategy::Reliable.requires_cross_validation());
    assert!(VerifyStrategy::Certified.requires_cross_validation());
    // Thorough does NOT — it runs portfolio, not cross-validation.
    assert!(!VerifyStrategy::Thorough.requires_cross_validation());
}

#[test]
fn certified_alone_requires_certificate_artifact() {
    assert!(VerifyStrategy::Certified.requires_certificate());
    assert!(!VerifyStrategy::Reliable.requires_certificate());
    assert!(!VerifyStrategy::Thorough.requires_certificate());
}

#[test]
fn thorough_reliable_certified_require_explicit_specs() {
    assert!(VerifyStrategy::Thorough.requires_explicit_specs());
    assert!(VerifyStrategy::Reliable.requires_explicit_specs());
    assert!(VerifyStrategy::Certified.requires_explicit_specs());
    assert!(!VerifyStrategy::Formal.requires_explicit_specs());
}

#[test]
fn proof_is_the_only_tactic_proof_strategy() {
    assert!(VerifyStrategy::Proof.requires_tactic_proof());
    for s in VerifyStrategy::LADDER {
        if s != VerifyStrategy::Proof {
            assert!(
                !s.requires_tactic_proof(),
                "{s:?} should not require tactic proof"
            );
        }
    }
}

#[test]
fn synthesize_is_the_only_synthesis_strategy() {
    assert!(VerifyStrategy::Synthesize.is_synthesis());
    for s in VerifyStrategy::LADDER {
        if s != VerifyStrategy::Synthesize {
            assert!(
                !s.is_synthesis(),
                "{s:?} should not be classified as synthesis"
            );
        }
    }
}

// -----------------------------------------------------------------------------
// Timeout multiplier — relative cost budget
// -----------------------------------------------------------------------------

#[test]
fn timeout_multipliers_reflect_cost_ordering() {
    assert_eq!(VerifyStrategy::Runtime.timeout_multiplier(), 0.0);
    assert_eq!(VerifyStrategy::Static.timeout_multiplier(), 0.0);
    assert_eq!(VerifyStrategy::Proof.timeout_multiplier(), 0.0);

    assert!(VerifyStrategy::Fast.timeout_multiplier() < VerifyStrategy::Formal.timeout_multiplier());
    assert!(VerifyStrategy::Formal.timeout_multiplier() < VerifyStrategy::Thorough.timeout_multiplier());
    assert!(VerifyStrategy::Thorough.timeout_multiplier() <= VerifyStrategy::Reliable.timeout_multiplier());
    assert!(VerifyStrategy::Reliable.timeout_multiplier() <= VerifyStrategy::Certified.timeout_multiplier());
    assert!(VerifyStrategy::Certified.timeout_multiplier() < VerifyStrategy::Synthesize.timeout_multiplier());
}

// -----------------------------------------------------------------------------
// Round-trip — as_str / from_attribute_value
// -----------------------------------------------------------------------------

#[test]
fn canonical_form_round_trips() {
    for s in VerifyStrategy::LADDER {
        let canonical = s.as_str();
        let parsed = VerifyStrategy::from_attribute_value(canonical)
            .expect("canonical form must parse");
        assert_eq!(parsed, s);
    }
}
