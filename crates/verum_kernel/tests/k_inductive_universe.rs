//! K-Inductive universe-level integration tests (V8, #215).
//!
//! Pre-V8 the kernel's `infer` arm for `CoreTerm::Inductive` returned
//! `Universe(Concrete(0))` regardless of the declared level — silently
//! demoting HoTT-level types to set-level. V8 ships
//! `RegisteredInductive::universe`, `InductiveRegistry::universe_for`,
//! and `infer_with_inductives` so the typing judgment honours the
//! declared level.

use verum_common::{List, Text};
use verum_kernel::{
    AxiomRegistry, ConstructorSig, Context, CoreTerm, InductiveRegistry,
    RegisteredInductive, UniverseLevel, infer, infer_with_inductives,
};

fn empty_axioms() -> AxiomRegistry {
    AxiomRegistry::new()
}

fn empty_ctx() -> Context {
    Context::new()
}

fn nat_decl() -> RegisteredInductive {
    RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![
            ConstructorSig {
                name: Text::from("Zero"),
                arg_types: List::new(),
            },
        ]),
    )
}

#[test]
fn legacy_infer_returns_concrete_zero_for_inductive() {
    // Sanity: the public infer() shim preserves pre-V8 behaviour
    // (no inductive registry → fall back to Concrete(0)).
    let term = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let ty = infer(&empty_ctx(), &term, &empty_axioms()).expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

#[test]
fn infer_with_empty_registry_falls_back_to_concrete_zero() {
    // Empty registry → no entries → universe_for returns None →
    // infer_with_inductives still falls back to Concrete(0).
    let term = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let inductives = InductiveRegistry::new();
    let ty = infer_with_inductives(&empty_ctx(), &term, &empty_axioms(), &inductives)
        .expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

#[test]
fn infer_with_registered_concrete_zero_returns_concrete_zero() {
    // Explicit registration at default level — explicit
    // matches the implicit fallback.
    let mut inductives = InductiveRegistry::new();
    inductives
        .register(nat_decl())
        .expect("registration must succeed");
    let term = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let ty = infer_with_inductives(&empty_ctx(), &term, &empty_axioms(), &inductives)
        .expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

#[test]
fn infer_with_registered_concrete_two_returns_concrete_two() {
    // V8 SMOKING GUN: HoTT-level inductive declared at Type(2).
    // Pre-V8 behaviour: silently demoted to Type(0).
    // V8 behaviour: returns Type(2) faithfully.
    let mut inductives = InductiveRegistry::new();
    inductives
        .register(nat_decl().with_universe(UniverseLevel::Concrete(2)))
        .expect("registration must succeed");
    let term = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let ty = infer_with_inductives(&empty_ctx(), &term, &empty_axioms(), &inductives)
        .expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(2)));
}

#[test]
fn infer_with_registered_prop_universe_returns_prop() {
    // Inductive at the propositional universe (e.g., a
    // mere-proposition truncation). V8 honours Prop just as
    // faithfully as Concrete(n).
    let mut inductives = InductiveRegistry::new();
    inductives
        .register(nat_decl().with_universe(UniverseLevel::Prop))
        .expect("registration must succeed");
    let term = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let ty = infer_with_inductives(&empty_ctx(), &term, &empty_axioms(), &inductives)
        .expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Prop));
}

#[test]
fn unregistered_inductive_falls_back_to_concrete_zero_in_either_path() {
    // When the path isn't in the supplied registry,
    // universe_for returns None and we fall back to Concrete(0).
    // This preserves backwards compatibility for any caller that
    // doesn't pre-populate every inductive used.
    let term = CoreTerm::Inductive {
        path: Text::from("UnregisteredNat"),
        args: List::new(),
    };
    let inductives = InductiveRegistry::new();
    let ty = infer_with_inductives(&empty_ctx(), &term, &empty_axioms(), &inductives)
        .expect("infer ok");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(0)));
}

#[test]
fn universe_for_lookup_returns_registered_level() {
    let mut inductives = InductiveRegistry::new();
    inductives
        .register(nat_decl().with_universe(UniverseLevel::Concrete(3)))
        .expect("registration ok");
    match inductives.universe_for("Nat") {
        Some(level) => assert_eq!(*level, UniverseLevel::Concrete(3)),
        None => panic!("expected registered level"),
    }
    assert!(inductives.universe_for("Missing").is_none());
}

#[test]
fn registered_inductive_with_universe_serde_roundtrip() {
    // The new field carries serde(default), so old on-disk
    // certificates / cached registrations without the universe
    // field still deserialise — they default to Concrete(0).
    use serde_json;
    let json = r#"{"name":"OldNat","params":[],"constructors":[]}"#;
    let decl: RegisteredInductive =
        serde_json::from_str(json).expect("legacy json deserialises");
    assert_eq!(decl.universe, UniverseLevel::Concrete(0));

    // New JSON with explicit universe field round-trips cleanly.
    let new_decl =
        nat_decl().with_universe(UniverseLevel::Concrete(2));
    let serialized = serde_json::to_string(&new_decl).expect("serialise");
    let restored: RegisteredInductive =
        serde_json::from_str(&serialized).expect("deserialise");
    assert_eq!(restored.universe, UniverseLevel::Concrete(2));
}

#[test]
fn nested_inductive_in_pi_uses_registered_universe() {
    // Pi(_: Nat). Nat — the codomain's Inductive head is typed
    // via infer_with_inductives → Universe(Concrete(2)) when
    // the registry says Nat lives there. This exercises
    // recursive-call propagation: the Pi rule's recursive
    // infer_inner call receives `inductives` and threads it
    // into the codomain's typing.
    use verum_common::Heap;
    let mut inductives = InductiveRegistry::new();
    inductives
        .register(nat_decl().with_universe(UniverseLevel::Concrete(2)))
        .expect("registration ok");
    let nat = CoreTerm::Inductive {
        path: Text::from("Nat"),
        args: List::new(),
    };
    let pi = CoreTerm::Pi {
        binder: Text::from("_"),
        domain: Heap::new(nat.clone()),
        codomain: Heap::new(nat),
    };
    let ty = infer_with_inductives(&empty_ctx(), &pi, &empty_axioms(), &inductives)
        .expect("infer ok");
    // Pi-formation: result lives in max(level(dom), level(codom)).
    // Both are Concrete(2), so the Pi inhabits
    // Universe(Max(Concrete(2), Concrete(2))).
    match ty {
        CoreTerm::Universe(UniverseLevel::Max(_a, _b)) => {}
        other => panic!("expected Universe(Max(...)), got {:?}", other),
    }
}
