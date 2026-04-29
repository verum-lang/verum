//! Red-team adversarial test suite for the foundational kernel
//! infrastructure (Ordinal + InfinityCategory + DiakrisisBridge).
//!
//! These tests systematically probe boundary conditions, hidden
//! invariants, and structural assumptions that could break under
//! adversarial input.  Each failure is either a bug to fix or an
//! intentional design choice that gets DOCUMENTED as a contract.

use verum_kernel::diakrisis_bridge::{BridgeAudit, BridgeId};
use verum_kernel::infinity_category::{
    CellLevel, InfinityCategory, InfinityMorphism, compose,
    compose_is_associative, identity_is_equivalence, is_equivalence_at,
};
use verum_kernel::ordinal::Ordinal;

// =============================================================================
// A. Ordinal arithmetic correctness attacks
// =============================================================================

#[test]
fn ordinal_lt_is_irreflexive() {
    // For every ordinal we test, α < α must be false.
    let candidates = vec![
        Ordinal::Finite(0),
        Ordinal::Finite(1),
        Ordinal::Finite(99),
        Ordinal::Omega,
        Ordinal::OmegaPlus(1),
        Ordinal::OmegaPlus(99),
        Ordinal::OmegaTimes(2),
        Ordinal::OmegaTimes(7),
        Ordinal::OmegaTimesPlus { k: 3, n: 5 },
        Ordinal::OmegaSquared,
        Ordinal::OmegaSquaredPlus(Box::new(Ordinal::Finite(7))),
        Ordinal::OmegaPow(3),
        Ordinal::OmegaPow(7),
        Ordinal::Kappa(1),
        Ordinal::Kappa(99),
    ];
    for a in &candidates {
        assert!(!a.lt(a), "ordinal {} must not be < itself", a.render());
    }
}

#[test]
fn ordinal_lt_is_antisymmetric() {
    // α < β AND β < α — should be impossible.
    let pairs = vec![
        (Ordinal::Finite(3), Ordinal::Finite(7)),
        (Ordinal::Finite(99), Ordinal::Omega),
        (Ordinal::Omega, Ordinal::OmegaPlus(1)),
        (Ordinal::OmegaPlus(3), Ordinal::OmegaTimes(2)),
        (Ordinal::OmegaTimes(2), Ordinal::OmegaSquared),
        (Ordinal::OmegaSquared, Ordinal::OmegaPow(3)),
        (Ordinal::OmegaPow(3), Ordinal::Kappa(1)),
        (Ordinal::Kappa(1), Ordinal::Kappa(2)),
    ];
    for (a, b) in &pairs {
        let ab = a.lt(b);
        let ba = b.lt(a);
        assert!(!(ab && ba),
            "antisymmetry violated: {} < {} AND {} < {}",
            a.render(), b.render(), b.render(), a.render());
    }
}

#[test]
fn ordinal_lt_is_transitive() {
    // α < β AND β < γ ⟹ α < γ.
    let chain = vec![
        Ordinal::Finite(0),
        Ordinal::Finite(7),
        Ordinal::Omega,
        Ordinal::OmegaPlus(3),
        Ordinal::OmegaTimes(2),
        Ordinal::OmegaTimesPlus { k: 5, n: 2 },
        Ordinal::OmegaSquared,
        Ordinal::OmegaPow(3),
        Ordinal::OmegaPow(7),
        Ordinal::Kappa(1),
        Ordinal::Kappa(99),
    ];
    for i in 0..chain.len() {
        for j in (i + 1)..chain.len() {
            for k in (j + 1)..chain.len() {
                let a = &chain[i];
                let b = &chain[j];
                let c = &chain[k];
                if a.lt(b) && b.lt(c) {
                    assert!(a.lt(c),
                        "transitivity violated: {} < {} < {}, but NOT {} < {}",
                        a.render(), b.render(), c.render(),
                        a.render(), c.render());
                }
            }
        }
    }
}

#[test]
fn ordinal_succ_strictly_greater() {
    // For every ordinal α, α < α.succ() must hold.
    let candidates = vec![
        Ordinal::Finite(0),
        Ordinal::Finite(99),
        Ordinal::Omega,
        Ordinal::OmegaPlus(1),
        Ordinal::OmegaTimes(3),
        Ordinal::OmegaSquared,
    ];
    for a in &candidates {
        let next = a.succ();
        assert!(a.lt(&next),
            "{} should be < {}.succ() = {}",
            a.render(), a.render(), next.render());
    }
}

#[test]
fn ordinal_sup_empty() {
    // Edge case: Sup of empty Vec.  Mathematically this should be the
    // smallest ordinal (0).  Our implementation: `parts.iter().all(p)`
    // on empty iter returns true vacuously — so empty Sup < anything.
    let empty_sup = Ordinal::Sup(vec![]);
    assert!(empty_sup.lt(&Ordinal::Finite(1)),
        "empty Sup should be < every nonzero ordinal");
    assert!(empty_sup.lt(&Ordinal::Omega));
}

#[test]
fn ordinal_sup_singleton_equals_member() {
    // Sup([α]) should behave like α for lt purposes.
    let omega_sup = Ordinal::Sup(vec![Ordinal::Omega]);
    assert!(omega_sup.lt(&Ordinal::OmegaPlus(1)));
    assert!(!omega_sup.lt(&Ordinal::Omega));
}

#[test]
fn ordinal_sup_nested() {
    // Nested Sup should behave structurally — Sup([Sup([Omega])]) < Kappa(1).
    let inner = Ordinal::Sup(vec![Ordinal::Omega]);
    let outer = Ordinal::Sup(vec![inner]);
    assert!(outer.lt(&Ordinal::Kappa(1)));
}

#[test]
fn ordinal_render_produces_distinct_outputs() {
    // Different ordinal shapes should produce different render strings.
    let renders = vec![
        Ordinal::Finite(0).render(),
        Ordinal::Finite(7).render(),
        Ordinal::Omega.render(),
        Ordinal::OmegaPlus(3).render(),
        Ordinal::OmegaTimes(2).render(),
        Ordinal::OmegaSquared.render(),
        Ordinal::OmegaPow(7).render(),
        Ordinal::Kappa(1).render(),
        Ordinal::Kappa(99).render(),
    ];
    let mut seen = std::collections::HashSet::new();
    for r in &renders {
        assert!(seen.insert(r.clone()),
            "render() collision on {:?}", r);
    }
}

#[test]
fn ordinal_finite_zero_not_regular_per_convention() {
    // Convention: 0 is not regular.  Documented invariant.
    assert!(!Ordinal::Finite(0).is_regular());
    // But Finite(1), Finite(2), ... are regular.
    assert!(Ordinal::Finite(1).is_regular());
    assert!(Ordinal::Finite(99).is_regular());
}

#[test]
fn ordinal_sup_not_regular_conservative() {
    // Sup is conservatively NOT regular (unless it happens to equal
    // a regular ordinal, but that requires deeper analysis).
    let sup = Ordinal::Sup(vec![Ordinal::Finite(2), Ordinal::Finite(5)]);
    assert!(!sup.is_regular());
}

#[test]
fn ordinal_omega_pow_equality() {
    // OmegaPow(2) should NOT equal OmegaSquared via Eq derivation —
    // they're different variants even though semantically equivalent.
    // This is an INTENTIONAL design choice: we use OmegaSquared for
    // ω² to give it a canonical render, but OmegaPow(2) is admissible
    // and treated as semantically equal under lt comparison.
    let omega_squared = Ordinal::OmegaSquared;
    let omega_pow_2 = Ordinal::OmegaPow(2);
    // They are NOT structurally equal (different enum variants).
    assert_ne!(omega_squared, omega_pow_2);
    // Under lt, neither is < the other (they should be equal-ranked).
    // Currently OmegaSquared is hardcoded < OmegaPow(3) but
    // OmegaSquared vs OmegaPow(2) interaction is not directly tested.
    // EDGE CASE: we only have OmegaPow for e ≥ 3 by convention.  The
    // module docstring says "Covers ω³, ω⁴, ..." so OmegaPow(2)
    // is technically out of contract.  Document this.
}

// =============================================================================
// B. InfinityCategory correctness attacks
// =============================================================================

#[test]
fn infinity_morphism_is_identity_strict() {
    // is_identity check requires source == target AND name starts with "id_".
    // An adversary could construct a non-identity morphism with name "id_X" —
    // we should reject if source != target even with name match.
    let pseudo_id = InfinityMorphism {
        name: verum_common::Text::from("id_X"),
        source: verum_common::Text::from("X"),
        target: verum_common::Text::from("Y"),  // DIFFERENT target
        cell: CellLevel::Morphism,
    };
    assert!(!pseudo_id.is_identity(),
        "morphism with id_X name but different source/target must NOT be identity");
}

#[test]
fn infinity_morphism_is_identity_name_required() {
    // Conversely, source == target with non-id_ name should NOT be identity.
    let endo = InfinityMorphism {
        name: verum_common::Text::from("endomorphism"),
        source: verum_common::Text::from("X"),
        target: verum_common::Text::from("X"),
        cell: CellLevel::Morphism,
    };
    assert!(!endo.is_identity(),
        "endomorphism without id_ name must NOT be classified as identity");
}

#[test]
fn compose_endomorphism_self() {
    // f: X → X composed with itself: should produce X → X.
    let f = InfinityMorphism {
        name: verum_common::Text::from("f"),
        source: verum_common::Text::from("X"),
        target: verum_common::Text::from("X"),
        cell: CellLevel::Morphism,
    };
    let ff = compose(&f, &f).unwrap();
    assert_eq!(ff.source, verum_common::Text::from("X"));
    assert_eq!(ff.target, verum_common::Text::from("X"));
}

#[test]
fn compose_with_identity_is_left_unit() {
    // id_Y ∘ f = f when f: X → Y.
    let f = InfinityMorphism {
        name: verum_common::Text::from("f"),
        source: verum_common::Text::from("X"),
        target: verum_common::Text::from("Y"),
        cell: CellLevel::Morphism,
    };
    let id_y = InfinityMorphism::identity("Y");
    let composed = compose(&f, &id_y).expect("typed composition");
    assert_eq!(composed.source, f.source);
    assert_eq!(composed.target, f.target);
}

#[test]
fn compose_associativity_at_higher_cell_level() {
    // 2-cell composition should still be strictly associative at the V0
    // surface (V1 will introduce associator 2-cells).
    let f = InfinityMorphism {
        name: verum_common::Text::from("α"),
        source: verum_common::Text::from("A"),
        target: verum_common::Text::from("B"),
        cell: CellLevel::TwoCell,
    };
    let g = InfinityMorphism {
        name: verum_common::Text::from("β"),
        source: verum_common::Text::from("B"),
        target: verum_common::Text::from("C"),
        cell: CellLevel::TwoCell,
    };
    let h = InfinityMorphism {
        name: verum_common::Text::from("γ"),
        source: verum_common::Text::from("C"),
        target: verum_common::Text::from("D"),
        cell: CellLevel::TwoCell,
    };
    assert!(compose_is_associative(&f, &g, &h),
        "V0 strict-associativity holds at level 2 (V1 will weaken to associator-up-to-2-cell)");
}

#[test]
fn truncation_of_truncation() {
    // τ_{≤2}(τ_{≤5}(C)) should equal τ_{≤2}(C) — truncation is idempotent.
    let c = InfinityCategory::at_canonical_universe("∞-Top", Ordinal::Omega);
    let trunc1 = c.truncate_at(Ordinal::Finite(5));
    let trunc2 = trunc1.truncate_at(Ordinal::Finite(2));
    assert_eq!(trunc2.level, Ordinal::Finite(2));
    // Universe is preserved.
    assert_eq!(trunc2.universe, c.universe);
}

// =============================================================================
// C. BridgeAudit invariant attacks
// =============================================================================

#[test]
fn bridge_audit_dedup_same_bridge_same_context() {
    let mut a = BridgeAudit::new();
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-A");
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-A");
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-A");
    assert_eq!(a.admits().len(), 1, "triple-dup must collapse to 1");
}

#[test]
fn bridge_audit_distinct_contexts_kept() {
    let mut a = BridgeAudit::new();
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-A");
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-B");
    a.record(BridgeId::ConfluenceOfModalRewrite, "ctx-C");
    assert_eq!(a.admits().len(), 3, "distinct contexts must all record");
}

#[test]
fn bridge_audit_decidable_iff_empty() {
    let a = BridgeAudit::new();
    assert!(a.is_decidable());
    let mut b = BridgeAudit::new();
    b.record(BridgeId::EpsMuTauWitness, "x");
    assert!(!b.is_decidable());
}

// =============================================================================
// D. Cross-cutting attacks
// =============================================================================

#[test]
fn id_x_at_finite_level_is_decidable() {
    // The MSFS-critical property: id_X is an (∞,n)-equivalence for
    // every finite n with NO bridge admit (fully decidable).
    for n in 0..10 {
        let mut audit = BridgeAudit::new();
        let id = InfinityMorphism::identity("X");
        let result = is_equivalence_at(
            &id, &Ordinal::Finite(n), &mut audit, "msfs-thm-5.1",
        );
        assert!(result, "id_X must be (∞,{})-equivalence", n);
        assert!(audit.is_decidable(),
            "id_X at level {} must be decidable (zero bridge admits)", n);
    }
}

#[test]
fn non_identity_at_omega_records_bridge() {
    // Non-identity morphism at limit-level requires bridge admit
    // (Theorem A.7 stabilisation).  This is the design boundary.
    let mut audit = BridgeAudit::new();
    let f = InfinityMorphism {
        name: verum_common::Text::from("f"),
        source: verum_common::Text::from("A"),
        target: verum_common::Text::from("B"),
        cell: CellLevel::Morphism,
    };
    is_equivalence_at(&f, &Ordinal::Omega, &mut audit, "non-id-omega");
    assert!(!audit.is_decidable(),
        "non-identity at limit level must invoke a bridge admit");
}

#[test]
fn identity_at_kappa_decidable() {
    // Identity at inaccessible level should STILL be decidable
    // (the κ-level admit only fires for non-identity morphisms).
    let mut audit = BridgeAudit::new();
    let id = InfinityMorphism::identity("X");
    is_equivalence_at(&id, &Ordinal::Kappa(1), &mut audit, "id-kappa");
    assert!(audit.is_decidable(),
        "identity at κ_1 should still be decidable — id is identity at every level");
}

// =============================================================================
// E. Cross-module integration
// =============================================================================

#[test]
fn ordinal_used_in_cell_level_higher() {
    // CellLevel::HigherCell embeds an Ordinal — make sure the embedding
    // round-trips through the level() projection.
    let omega_cell = CellLevel::HigherCell(Ordinal::Omega);
    assert_eq!(omega_cell.level(), Ordinal::Omega);
    let kappa_cell = CellLevel::HigherCell(Ordinal::Kappa(1));
    assert_eq!(kappa_cell.level(), Ordinal::Kappa(1));
}

#[test]
fn category_truncation_uses_ordinal_lt() {
    // Truncating at level n keeps cells with level < n.
    let c = InfinityCategory::at_canonical_universe("Cat", Ordinal::Omega);
    let trunc_2 = c.truncate_at(Ordinal::Finite(2));
    let trunc_omega = c.truncate_at(Ordinal::Omega);
    assert!(trunc_2.level.lt(&trunc_omega.level));
}

#[test]
fn equivalence_audit_grows_monotonically_under_compose_calls() {
    // Each is_equivalence_at call may add bridge admits to the audit.
    // Different calls should ADD entries, never remove them.
    let mut audit = BridgeAudit::new();
    let f = InfinityMorphism {
        name: verum_common::Text::from("f"),
        source: verum_common::Text::from("A"),
        target: verum_common::Text::from("B"),
        cell: CellLevel::Morphism,
    };
    let g = InfinityMorphism {
        name: verum_common::Text::from("g"),
        source: verum_common::Text::from("B"),
        target: verum_common::Text::from("C"),
        cell: CellLevel::Morphism,
    };
    is_equivalence_at(&f, &Ordinal::Omega, &mut audit, "ctx-f");
    let count_after_f = audit.admits().len();
    is_equivalence_at(&g, &Ordinal::Omega, &mut audit, "ctx-g");
    let count_after_g = audit.admits().len();
    assert!(count_after_g >= count_after_f,
        "audit count must grow monotonically");
}
