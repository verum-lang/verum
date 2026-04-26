//! K-Coord-Cite kernel-rule integration tests (V8 #227).
//!
//! Per VVA §A.Z.5 item 2: a theorem at coordinate (Fw, ν, τ)
//! may cite an axiom at coordinate (Fw', ν', τ') only when
//! ν' ≤ ν (lex on OrdinalDepth). Higher-tier citations are
//! rejected unless the calling module imports the κ-tier-jump
//! extension (`@require_extension(vfe_3)` — VVA-3 K-Universe-
//! Ascent).

use verum_common::Text;
use verum_kernel::{
    AxiomRegistry, FrameworkId, KernelCoord, KernelError, OrdinalDepth, check_coord_cite,
};

fn fw(name: &str) -> FrameworkId {
    FrameworkId {
        framework: Text::from(name),
        citation: Text::from("test"),
    }
}

// =============================================================================
// check_coord_cite — direct rule invariants
// =============================================================================

#[test]
fn same_coord_cite_admitted() {
    let coord = KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(0));
    let res = check_coord_cite(&coord, &coord, &Text::from("self_cite"), false);
    assert!(res.is_ok());
}

#[test]
fn lower_axiom_cite_admitted() {
    // theorem at ν=2 cites axiom at ν=1 → admitted (axiom.ν ≤ theorem.ν).
    let theorem_coord =
        KernelCoord::canonical(Text::from("hott"), OrdinalDepth::finite(2));
    let axiom_coord =
        KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(1));
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("set_axiom"),
        false,
    );
    assert!(res.is_ok());
}

#[test]
fn higher_axiom_cite_rejected_without_tier_jump() {
    // theorem at ν=1 tries to cite axiom at ν=2 → rejected.
    let theorem_coord =
        KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(1));
    let axiom_coord =
        KernelCoord::canonical(Text::from("hott"), OrdinalDepth::finite(2));
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("hott_axiom"),
        false,
    );
    match res {
        Err(KernelError::CoordViolation {
            axiom_name,
            theorem_fw,
            theorem_nu,
            axiom_fw,
            axiom_nu,
        }) => {
            assert_eq!(axiom_name.as_str(), "hott_axiom");
            assert_eq!(theorem_fw.as_str(), "set_level");
            assert_eq!(theorem_nu.as_str(), "1");
            assert_eq!(axiom_fw.as_str(), "hott");
            assert_eq!(axiom_nu.as_str(), "2");
        }
        other => panic!("expected CoordViolation, got {:?}", other),
    }
}

#[test]
fn higher_axiom_cite_admitted_under_tier_jump() {
    // theorem at ν=1 cites axiom at ν=2 with allow_tier_jump=true
    // (VVA-3 K-Universe-Ascent) → admitted.
    let theorem_coord =
        KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(1));
    let axiom_coord =
        KernelCoord::canonical(Text::from("hott"), OrdinalDepth::finite(2));
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("hott_axiom"),
        true,
    );
    assert!(
        res.is_ok(),
        "tier-jump must admit higher-ν citation: {:?}",
        res,
    );
}

#[test]
fn omega_axiom_cite_from_set_theorem_rejected() {
    // theorem at ν=3 tries to cite axiom at ν=ω → rejected
    // (finite < ω lex).
    let theorem_coord =
        KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(3));
    let axiom_coord = KernelCoord::canonical(Text::from("lurie_htt"), OrdinalDepth::omega());
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("lurie_axiom"),
        false,
    );
    assert!(matches!(res, Err(KernelError::CoordViolation { .. })));
}

#[test]
fn omega_theorem_cites_finite_axiom_admitted() {
    // theorem at ν=ω cites axiom at ν=5 → admitted.
    let theorem_coord = KernelCoord::canonical(Text::from("lurie_htt"), OrdinalDepth::omega());
    let axiom_coord =
        KernelCoord::canonical(Text::from("set_level"), OrdinalDepth::finite(5));
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("set_axiom"),
        false,
    );
    assert!(res.is_ok());
}

#[test]
fn omega_plus_one_theorem_cites_omega_axiom_admitted() {
    // theorem at ν=ω+1 cites axiom at ν=ω → admitted.
    let theorem_coord = KernelCoord::canonical(
        Text::from("baez_dolan"),
        OrdinalDepth { omega_coeff: 1, finite_offset: 1 },
    );
    let axiom_coord = KernelCoord::canonical(Text::from("lurie_htt"), OrdinalDepth::omega());
    let res = check_coord_cite(
        &theorem_coord,
        &axiom_coord,
        &Text::from("lurie_axiom"),
        false,
    );
    assert!(res.is_ok());
}

// =============================================================================
// register_with_coord — populates the registry's coord field
// =============================================================================

#[test]
fn register_with_coord_attaches_coord_to_entry() {
    let mut reg = AxiomRegistry::new();
    let coord = KernelCoord::canonical(Text::from("test_fw"), OrdinalDepth::finite(2));
    let ty = verum_kernel::CoreTerm::Inductive {
        path: Text::from("Unit"),
        args: verum_common::List::new(),
    };
    reg.register_with_coord(
        Text::from("a1"),
        ty,
        fw("test_fw"),
        coord.clone(),
    )
    .expect("admit");
    use verum_common::Maybe;
    match reg.get("a1") {
        Maybe::Some(entry) => {
            assert_eq!(entry.coord.as_ref(), Some(&coord));
        }
        Maybe::None => panic!("entry not registered"),
    }
}

#[test]
fn register_with_coord_rejects_uip_shape() {
    use verum_common::{Heap, List};
    use verum_kernel::CoreTerm;
    use verum_kernel::UniverseLevel;
    let mut reg = AxiomRegistry::new();
    // Build the precise UIP shape — the strict admission gate
    // must reject before populating the coord field.
    let path_inner = CoreTerm::PathTy {
        carrier: Heap::new(CoreTerm::Var(Text::from("A"))),
        lhs: Heap::new(CoreTerm::Var(Text::from("a"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("b"))),
    };
    let path_outer = CoreTerm::PathTy {
        carrier: Heap::new(path_inner.clone()),
        lhs: Heap::new(CoreTerm::Var(Text::from("p"))),
        rhs: Heap::new(CoreTerm::Var(Text::from("q"))),
    };
    let pi_q = CoreTerm::Pi {
        binder: Text::from("q"),
        domain: Heap::new(path_inner.clone()),
        codomain: Heap::new(path_outer),
    };
    let pi_p = CoreTerm::Pi {
        binder: Text::from("p"),
        domain: Heap::new(path_inner.clone()),
        codomain: Heap::new(pi_q),
    };
    let pi_b = CoreTerm::Pi {
        binder: Text::from("b"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(pi_p),
    };
    let pi_a = CoreTerm::Pi {
        binder: Text::from("a"),
        domain: Heap::new(CoreTerm::Var(Text::from("A"))),
        codomain: Heap::new(pi_b),
    };
    let uip = CoreTerm::Pi {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(pi_a),
    };
    let coord = KernelCoord::canonical(Text::from("test_fw"), OrdinalDepth::finite(0));
    let res = reg.register_with_coord(
        Text::from("uip_attempt"),
        uip,
        fw("test_fw"),
        coord,
    );
    assert!(matches!(res, Err(KernelError::UipForbidden(_))));
    use verum_common::Maybe;
    assert!(matches!(reg.get("uip_attempt"), Maybe::None));
}

#[test]
fn legacy_register_leaves_coord_none() {
    let mut reg = AxiomRegistry::new();
    let ty = verum_kernel::CoreTerm::Inductive {
        path: Text::from("Unit"),
        args: verum_common::List::new(),
    };
    reg.register(Text::from("legacy"), ty, fw("test")).expect("admit");
    use verum_common::Maybe;
    match reg.get("legacy") {
        Maybe::Some(entry) => {
            assert!(entry.coord.is_none(), "legacy register leaves coord None");
        }
        Maybe::None => panic!("entry not registered"),
    }
}

// =============================================================================
// KernelCoord builders
// =============================================================================

#[test]
fn canonical_builder_sets_tau_true() {
    let c = KernelCoord::canonical(Text::from("set"), OrdinalDepth::finite(1));
    assert!(c.tau);
}

#[test]
fn staged_builder_sets_tau_false() {
    let c = KernelCoord::staged(Text::from("set"), OrdinalDepth::finite(1));
    assert!(!c.tau);
}

#[test]
fn coord_serde_roundtrip() {
    use serde_json;
    let c = KernelCoord::canonical(
        Text::from("lurie_htt"),
        OrdinalDepth { omega_coeff: 1, finite_offset: 2 },
    );
    let s = serde_json::to_string(&c).expect("serialise");
    let restored: KernelCoord = serde_json::from_str(&s).expect("deserialise");
    assert_eq!(restored, c);
}

#[test]
fn pre_v8_axiom_serde_lacks_coord_field() {
    use serde_json;
    use verum_kernel::RegisteredAxiom;
    // Pre-V8 #227 JSON without `coord` field must deserialise
    // as None (preserving on-disk certificate compatibility).
    let json = r#"{
        "name": "old_axiom",
        "ty": {"Inductive": {"path": "Unit", "args": []}},
        "framework": {"framework": "test", "citation": "test"}
    }"#;
    let entry: RegisteredAxiom = serde_json::from_str(json).expect("legacy parse");
    assert!(entry.coord.is_none());
    assert!(entry.body.is_none());
}
