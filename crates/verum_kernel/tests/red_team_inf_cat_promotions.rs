//! Red-team adversarial sweep for the V0 Yoneda / Cartesian-fibration /
//! Adjoint-functor kernel modules.
//!
//! Five attack categories:
//!
//!   A. **Precondition bypass** — declare witness flags pathologically
//!      and verify the kernel-level decision predicates reject the
//!      inconsistent inputs.
//!   B. **Algebraic identities** — composition associativity / unit
//!      laws on adjunctions; Yoneda-lemma natural-isomorphism endpoints.
//!   C. **Universe-ascent monotonicity** — `presheaf_category` must
//!      strictly increase the universe under κ-tier ascent.
//!   D. **Hierarchy crossing** — adjunctions across κ-tiers,
//!      Grothendieck-construction κ-preservation under unstraightening.
//!   E. **Dual/self-dual symmetry** — `BuildLeftOfRight` vs
//!      `BuildRightOfLeft` produce the same adjunction modulo
//!      naming convention.

use verum_kernel::adjoint_functor::{
    AdjunctionDirection, SaftPreconditions, build_adjunction,
    compose_adjunctions, left_adjoint_exists, right_adjoint_exists,
    triangle_identities_witness,
};
use verum_kernel::cartesian_fibration::{
    CartesianFibration, CartesianMorphism, build_straightening_equivalence,
    fibration_is_unstraightened, is_cartesian, unstraighten_to_grothendieck,
};
use verum_kernel::grothendieck::{SIndexedDiagram, preserves_accessibility};
use verum_kernel::infinity_category::InfinityCategory;
use verum_kernel::ordinal::Ordinal;
use verum_kernel::yoneda::{
    Presheaf, build_kan_extension, kan_extension_unit_witness,
    presheaf_category, yoneda_embedding, yoneda_lemma,
};
use verum_common::Text;

// =============================================================================
// A. Precondition bypass attacks
// =============================================================================

#[test]
fn a01_left_adjoint_rejects_each_missing_precondition() {
    let base = SaftPreconditions::fully_satisfied("L");
    // Each of the three preconditions is independently load-bearing.
    let mut p1 = base.clone();
    p1.source_presentable = false;
    assert!(!left_adjoint_exists(&p1));
    let mut p2 = base.clone();
    p2.target_presentable = false;
    assert!(!left_adjoint_exists(&p2));
    let mut p3 = base.clone();
    p3.preserves_small_colimits = false;
    assert!(!left_adjoint_exists(&p3));
}

#[test]
fn a02_right_adjoint_rejects_each_missing_precondition() {
    let base = SaftPreconditions::fully_satisfied("R");
    let mut p1 = base.clone();
    p1.source_presentable = false;
    assert!(!right_adjoint_exists(&p1));
    let mut p2 = base.clone();
    p2.target_presentable = false;
    assert!(!right_adjoint_exists(&p2));
    let mut p3 = base.clone();
    p3.preserves_small_limits_and_accessible = false;
    assert!(!right_adjoint_exists(&p3));
}

#[test]
fn a03_build_kan_extension_rejects_each_missing_precondition() {
    // f not fully faithful: should fail.
    assert!(build_kan_extension("f", "p", "C", "D", "E", false, true).is_none());
    // target without colimits: should fail.
    assert!(build_kan_extension("f", "p", "C", "D", "E", true, false).is_none());
    // both missing: should fail.
    assert!(build_kan_extension("f", "p", "C", "D", "E", false, false).is_none());
    // both present: should succeed.
    assert!(build_kan_extension("f", "p", "C", "D", "E", true, true).is_some());
}

#[test]
fn a04_unstraighten_propagates_empty_diagram_failure() {
    let diagram = SIndexedDiagram::finite("D", "B", vec![], Ordinal::Kappa(1));
    assert!(unstraighten_to_grothendieck(&diagram).is_none(),
        "Un must propagate Grothendieck's empty-diagram rejection");
}

#[test]
fn a05_fibration_is_unstraightened_demands_both_witnesses() {
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));
    // Only one of the two witness flags is enough to defeat the predicate.
    let p_no_cart = CartesianFibration::new(
        "p", e.clone(), c.clone(), false, true,
    );
    let p_no_cocart = CartesianFibration::new(
        "p", e.clone(), c.clone(), true, false,
    );
    let p_full = CartesianFibration::new(
        "p", e, c, true, true,
    );
    assert!(!fibration_is_unstraightened(&p_no_cart));
    assert!(!fibration_is_unstraightened(&p_no_cocart));
    assert!(fibration_is_unstraightened(&p_full));
}

// =============================================================================
// B. Algebraic identities
// =============================================================================

#[test]
fn b01_compose_adjunctions_preserves_coherence() {
    let pre = SaftPreconditions::fully_satisfied("L");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));
    let cd = build_adjunction("L_CD", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let de = build_adjunction("L_DE", &d, &e, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let ce = compose_adjunctions(&cd, &de).unwrap();
    assert!(ce.is_coherent(),
        "Adjunction composition must preserve unit + counit + triangle identities");
}

#[test]
fn b02_compose_adjunctions_associates() {
    // (cd ∘ de) ∘ ef should equal cd ∘ (de ∘ ef) up to source/target identity.
    let pre = SaftPreconditions::fully_satisfied("L");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));
    let f = InfinityCategory::at_canonical_universe("F", Ordinal::Finite(1));
    let cd = build_adjunction("L_CD", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let de = build_adjunction("L_DE", &d, &e, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let ef = build_adjunction("L_EF", &e, &f, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();

    let lhs = compose_adjunctions(&compose_adjunctions(&cd, &de).unwrap(), &ef).unwrap();
    let rhs = compose_adjunctions(&cd, &compose_adjunctions(&de, &ef).unwrap()).unwrap();

    assert_eq!(lhs.source_category, rhs.source_category,
        "Adjunction composition source must be associative");
    assert_eq!(lhs.target_category, rhs.target_category,
        "Adjunction composition target must be associative");
    assert!(lhs.is_coherent());
    assert!(rhs.is_coherent());
}

#[test]
fn b03_yoneda_lemma_endpoint_consistency() {
    // For any presheaf p and object x, the LHS should mention "Hom_PSh"
    // and the RHS should be exactly p(x).
    let p = Presheaf::non_representable("Set", "F");
    let lemma = yoneda_lemma("X", &p);
    assert!(lemma.lhs_name.as_str().starts_with("Hom_"));
    assert_eq!(lemma.rhs_name.as_str(), "F(X)");
    assert!(lemma.is_natural_isomorphism);
}

#[test]
fn b04_yoneda_embedding_target_is_presheaf_category() {
    let c = InfinityCategory::at_canonical_universe("Set", Ordinal::Finite(1));
    let y = yoneda_embedding(&c);
    let psh = presheaf_category(&c);
    assert_eq!(y.target_category, psh,
        "Yoneda embedding's target must literally equal PSh(C)");
}

#[test]
fn b05_kan_extension_unit_witness_holds_on_built_extensions() {
    let ext = build_kan_extension("f", "p", "C", "D", "E", true, true).unwrap();
    assert!(kan_extension_unit_witness(&ext),
        "Kan extension's unit must be witnessed when build succeeded");
}

// =============================================================================
// C. Universe-ascent monotonicity
// =============================================================================

#[test]
fn c01_presheaf_category_strict_universe_ascent_for_kappa() {
    // For κ-tier base categories, PSh(C) must live strictly higher.
    for n in 0..5_u32 {
        let c = InfinityCategory {
            name: Text::from(format!("C_{}", n)),
            level: Ordinal::Finite(1),
            universe: Ordinal::Kappa(n),
        };
        let psh = presheaf_category(&c);
        assert!(c.universe.lt(&psh.universe),
            "PSh(C) at κ_{} must live strictly above C", n);
        assert_eq!(psh.universe, Ordinal::Kappa(n + 1),
            "Ascent on κ_n must produce κ_{{n+1}}");
    }
}

#[test]
fn c02_presheaf_category_below_kappa_jumps_to_kappa_0() {
    // For finite/ω-tier base categories, PSh(C) jumps directly to κ_0.
    let c = InfinityCategory {
        name: Text::from("C"),
        level: Ordinal::Finite(1),
        universe: Ordinal::Omega,
    };
    let psh = presheaf_category(&c);
    assert_eq!(psh.universe, Ordinal::Kappa(0),
        "Sub-κ ascent must land at κ_0 (the first inaccessible)");
}

#[test]
fn c03_presheaf_category_preserves_level() {
    // The cell-level (n in (∞, n)) is independent of universe ascent —
    // PSh(C) and C must share a level.
    for level in [Ordinal::Finite(1), Ordinal::Finite(2), Ordinal::Omega] {
        let c = InfinityCategory {
            name: Text::from("C"),
            level: level.clone(),
            universe: Ordinal::Kappa(1),
        };
        let psh = presheaf_category(&c);
        assert_eq!(psh.level, level,
            "PSh(C) must share C's cell-level");
    }
}

// =============================================================================
// D. Hierarchy crossing
// =============================================================================

#[test]
fn d01_unstraightening_preserves_kappa_accessibility() {
    // For any κ-level and a well-formed C-indexed diagram, the
    // resulting Grothendieck construction must inherit the κ.
    for k in 0..3_u32 {
        let diagram = SIndexedDiagram::finite(
            "D",
            "B",
            vec![
                (Text::from("b0"), Text::from("D_b0")),
                (Text::from("b1"), Text::from("D_b1")),
            ],
            Ordinal::Kappa(k),
        );
        let g = unstraighten_to_grothendieck(&diagram).unwrap();
        assert!(preserves_accessibility(&diagram, &g),
            "Un must inherit κ_{}-accessibility", k);
    }
}

#[test]
fn d02_compose_adjunctions_takes_min_level() {
    // When composing adjunctions of different levels, the result lives
    // at the min — composition cannot rise above either.
    let pre = SaftPreconditions::fully_satisfied("L");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));

    let cd = build_adjunction("L_CD", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let mut de = build_adjunction("L_DE", &d, &e, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    de.adjunction_level = Ordinal::Finite(0);

    let ce = compose_adjunctions(&cd, &de).unwrap();
    assert_eq!(ce.adjunction_level, Ordinal::Finite(0),
        "Composition level must be ≤ min(operand levels)");
}

#[test]
fn d03_unstraightening_fails_below_minimum_diagram_size() {
    let diagram = SIndexedDiagram::finite("D", "B", vec![], Ordinal::Kappa(1));
    assert!(unstraighten_to_grothendieck(&diagram).is_none());
}

#[test]
fn d04_compose_adjunctions_rejects_mismatched_categories() {
    let pre = SaftPreconditions::fully_satisfied("L");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));
    let cd = build_adjunction("L_CD", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let ec = build_adjunction("L_EC", &e, &c, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    // cd has target=D; ec has source=E.  Composition cd ∘ ec fails.
    assert!(compose_adjunctions(&cd, &ec).is_none(),
        "Adjunction composition must reject D ≠ E mismatch");
}

// =============================================================================
// E. Dual / self-dual symmetry
// =============================================================================

#[test]
fn e01_adjunction_directions_swap_left_right() {
    // BuildRightOfLeft places given on the left; BuildLeftOfRight on the right.
    let pre = SaftPreconditions::fully_satisfied("F");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));

    let right_of_left = build_adjunction(
        "F", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft,
    ).unwrap();
    let left_of_right = build_adjunction(
        "F", &c, &d, &pre, AdjunctionDirection::BuildLeftOfRight,
    ).unwrap();

    // Same given functor name appears in opposite slots.
    assert_eq!(right_of_left.left_functor.as_str(), "F");
    assert_eq!(left_of_right.right_functor.as_str(), "F");
    assert_ne!(right_of_left.left_functor, left_of_right.left_functor);
    assert_ne!(right_of_left.right_functor, left_of_right.right_functor);
    // Both are coherent.
    assert!(right_of_left.is_coherent());
    assert!(left_of_right.is_coherent());
}

#[test]
fn e02_yoneda_embedding_is_idempotent_under_repeated_construction() {
    // yoneda_embedding(c) is a deterministic function — calling twice
    // produces equal results.  Equivalent to "no hidden state".
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let y1 = yoneda_embedding(&c);
    let y2 = yoneda_embedding(&c);
    assert_eq!(y1, y2,
        "Yoneda embedding must be a deterministic function of C");
}

#[test]
fn e03_straightening_equivalence_is_deterministic() {
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let st1 = build_straightening_equivalence(&c);
    let st2 = build_straightening_equivalence(&c);
    assert_eq!(st1, st2);
}

#[test]
fn e04_is_cartesian_decision_matches_witness_flag() {
    let p = CartesianFibration::new(
        "p",
        InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1)),
        InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1)),
        true, true,
    );
    for flag in [true, false] {
        let f = CartesianMorphism {
            name: Text::from("f"),
            fibration_name: Text::from("p"),
            source: Text::from("e'"),
            target: Text::from("e"),
            is_p_cartesian: flag,
        };
        assert_eq!(is_cartesian(&p, &f), flag,
            "is_cartesian must agree with the morphism's witness flag");
    }
}

#[test]
fn e05_triangle_identities_propagate_through_composition() {
    // If both operand adjunctions satisfy their triangle identities,
    // the composition does too.  If one breaks, so does the result.
    let pre = SaftPreconditions::fully_satisfied("L");
    let c = InfinityCategory::at_canonical_universe("C", Ordinal::Finite(1));
    let d = InfinityCategory::at_canonical_universe("D", Ordinal::Finite(1));
    let e = InfinityCategory::at_canonical_universe("E", Ordinal::Finite(1));

    let cd = build_adjunction("L_CD", &c, &d, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    let mut de = build_adjunction("L_DE", &d, &e, &pre, AdjunctionDirection::BuildRightOfLeft).unwrap();
    de.triangle_identities_hold = false;

    let ce = compose_adjunctions(&cd, &de).unwrap();
    assert!(!triangle_identities_witness(&ce),
        "Triangle-identity failure must propagate through composition");
}
