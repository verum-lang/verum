//! δ-reduction (axiom unfolding) integration tests (V8, #223).
//!
//! ships `normalize_with_axioms` and
//! `definitional_eq_with_axioms` — the δ-aware companions to V8
//! #216's β-only `normalize` / `definitional_eq`. δ unfolds
//! transparent **definitions** (registered with non-None body
//! per `AxiomRegistry::register_definition`); opaque **postulates**
//! (no body) remain neutral.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, CoreTerm, FrameworkId, UniverseLevel, definitional_eq,
    definitional_eq_with_axioms, normalize, normalize_with_axioms,
};

fn fw() -> FrameworkId {
    FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    }
}

fn nat_ind() -> CoreTerm {
    CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    }
}

// =============================================================================
// register_definition contract
// =============================================================================

#[test]
fn register_definition_stores_body() {
    // A simple identity-on-Nat definition: `def IdNat : Nat → Nat
    // := λx. x`. The kernel stores the body for later δ-unfolding.
    let mut reg = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let id_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(nat_ind()),
    };
    let res = reg.register_definition(
        Text::from("IdNat"),
        id_lam.clone(),
        id_ty,
        fw(),
    );
    assert!(res.is_ok());
    use verum_common::Maybe;
    match reg.get("IdNat") {
        Maybe::Some(entry) => {
            assert_eq!(entry.body, Some(id_lam), "body must be stored verbatim");
        }
        Maybe::None => panic!("definition not registered"),
    }
}

#[test]
fn register_definition_rejects_uip_shape_typed_body() {
    // Even definitions can't be typed at the UIP shape (the
    // shape-rejector preserves the cubical-univalence soundness
    // gate at every entry point).
    use verum_common::Heap;
    let mut reg = AxiomRegistry::new();
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
    let uip_ty = CoreTerm::Pi {
        binder: Text::from("A"),
        domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(0))),
        codomain: Heap::new(pi_a),
    };
    let body = nat_ind(); // arbitrary body — UIP gate fires on ty.
    let res = reg.register_definition(Text::from("uip_def"), body, uip_ty, fw());
    assert!(matches!(
        res,
        Err(verum_kernel::KernelError::UipForbidden(_))
    ));
}

// =============================================================================
// normalize_with_axioms — δ-unfolding
// =============================================================================

#[test]
fn normalize_with_axioms_unfolds_transparent_definition() {
    // `IdNat : Nat → Nat := λx. x`. References to IdNat unfold
    // to the body during normalisation.
    let mut reg = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("x"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("x"))),
    };
    let id_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(nat_ind()),
    };
    reg.register_definition(Text::from("IdNat"), id_lam.clone(), id_ty.clone(), fw())
        .expect("register ok");
    // Term: `IdNat` (an Axiom node referencing the definition).
    let term = CoreTerm::Axiom {
        name: Text::from("IdNat"),
        ty: Heap::new(id_ty),
        framework: fw(),
    };
    let normalised = normalize_with_axioms(&term, &reg);
    // δ unfolds to the body, then β has nothing more to do
    // since the body is already a normal-form Lam.
    assert_eq!(normalised, id_lam, "δ unfolded to the stored body");
}

#[test]
fn normalize_with_axioms_leaves_opaque_postulate_neutral() {
    // Postulates (body = None) remain neutral under δ.
    let mut reg = AxiomRegistry::new();
    reg.register(Text::from("opaque_pos"), nat_ind(), fw())
        .expect("register ok");
    let term = CoreTerm::Axiom {
        name: Text::from("opaque_pos"),
        ty: Heap::new(nat_ind()),
        framework: fw(),
    };
    let normalised = normalize_with_axioms(&term, &reg);
    assert_eq!(
        normalised,
        CoreTerm::Axiom {
            name: Text::from("opaque_pos"),
            ty: Heap::new(nat_ind()),
            framework: fw(),
        },
        "postulate must stay neutral",
    );
}

#[test]
fn normalize_with_axioms_unfolds_inside_compound_term() {
    // `App(IdNat, x)` should unfold IdNat first → `App(λy. y, x)`,
    // then β-reduce → `x`.
    let mut reg = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    let id_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(nat_ind()),
    };
    reg.register_definition(Text::from("IdNat"), id_lam, id_ty.clone(), fw())
        .expect("register ok");
    let id_axiom = CoreTerm::Axiom {
        name: Text::from("IdNat"),
        ty: Heap::new(id_ty),
        framework: fw(),
    };
    let term = CoreTerm::App(
        Heap::new(id_axiom),
        Heap::new(CoreTerm::Var(Text::from("x"))),
    );
    let normalised = normalize_with_axioms(&term, &reg);
    assert_eq!(normalised, CoreTerm::Var(Text::from("x")));
}

#[test]
fn normalize_legacy_does_not_unfold() {
    // The pre-V8 `normalize` (no axiom registry) treats every
    // Axiom node as opaque. Sanity: existing β behaviour is
    // preserved under the registry-free entry point.
    let term = CoreTerm::Axiom {
        name: Text::from("would_unfold"),
        ty: Heap::new(nat_ind()),
        framework: fw(),
    };
    let normalised = normalize(&term);
    // No unfolding under the legacy entry point.
    assert_eq!(normalised, term);
}

// =============================================================================
// definitional_eq_with_axioms
// =============================================================================

#[test]
fn definitional_eq_with_axioms_handles_delta_equivalence() {
    // `IdNat x ≡_βδ x` once IdNat unfolds + β-reduces.
    let mut reg = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    let id_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(nat_ind()),
    };
    reg.register_definition(Text::from("IdNat"), id_lam, id_ty.clone(), fw())
        .expect("register ok");
    let id_axiom = CoreTerm::Axiom {
        name: Text::from("IdNat"),
        ty: Heap::new(id_ty),
        framework: fw(),
    };
    let app = CoreTerm::App(
        Heap::new(id_axiom),
        Heap::new(CoreTerm::Var(Text::from("x"))),
    );
    let bare = CoreTerm::Var(Text::from("x"));
    assert!(definitional_eq_with_axioms(&app, &bare, &reg));
    // Pre-V8 β-only definitional_eq does NOT see them as equal
    // (no δ-unfold) — sanity that the new entry point is
    // strictly more permissive.
    assert!(!definitional_eq(&app, &bare));
}

#[test]
fn definitional_eq_with_axioms_preserves_distinct_terms() {
    // Sanity: genuinely-different terms remain distinct.
    let reg = AxiomRegistry::new();
    let a = CoreTerm::Var(Text::from("a"));
    let b = CoreTerm::Var(Text::from("b"));
    assert!(!definitional_eq_with_axioms(&a, &b, &reg));
}

#[test]
fn definitional_eq_with_axioms_handles_byte_identical_terms() {
    let reg = AxiomRegistry::new();
    let t = CoreTerm::Var(Text::from("z"));
    assert!(definitional_eq_with_axioms(&t, &t, &reg));
}

#[test]
fn delta_reduces_axiom_referenced_inside_pi() {
    // `Π(_: IdNat). Nat` → δ-reduces inside the domain.
    // After δ, the domain is the Lam itself (since IdNat unfolds).
    let mut reg = AxiomRegistry::new();
    let id_lam = CoreTerm::Lam {
        binder: Text::from("y"),
        domain: Heap::new(nat_ind()),
        body: Heap::new(CoreTerm::Var(Text::from("y"))),
    };
    let id_ty = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat_ind()),
        codomain: Heap::new(nat_ind()),
    };
    reg.register_definition(Text::from("IdNat"), id_lam.clone(), id_ty.clone(), fw())
        .expect("register ok");
    let id_axiom = CoreTerm::Axiom {
        name: Text::from("IdNat"),
        ty: Heap::new(id_ty),
        framework: fw(),
    };
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(id_axiom),
        codomain: Heap::new(nat_ind()),
    };
    let normalised = normalize_with_axioms(&pi, &reg);
    let expected = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(id_lam),
        codomain: Heap::new(nat_ind()),
    };
    assert_eq!(normalised, expected);
}

// =============================================================================
// Backwards-compat: serde default for `body`
// =============================================================================

#[test]
fn pre_v8_axiom_serde_lacks_body_field() {
    use serde_json;
    use verum_kernel::RegisteredAxiom;
    // Pre-V8 JSON without `body` field must deserialise as
    // None (opaque postulate), preserving on-disk certificate
    // compatibility.
    let json = r#"{
        "name": "old_axiom",
        "ty": {"Inductive": {"path": "Nat", "args": []}},
        "framework": {"framework": "test", "citation": "test"}
    }"#;
    let entry: RegisteredAxiom = serde_json::from_str(json).expect("legacy parse");
    assert_eq!(entry.body, None);
}
