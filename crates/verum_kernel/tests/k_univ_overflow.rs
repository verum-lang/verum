//! K-Univ universe-level-overflow regression tests (V8, B1, #207).
//!
//! Pre-V8 the kernel's universe-typing rule used
//! `Concrete(n).saturating_add(1)` to compute the successor level.
//! At `u32::MAX` the saturation silently returned `u32::MAX`, so the
//! inference rule was effectively `Universe(Concrete(MAX)) :
//! Universe(Concrete(MAX))` — type-in-type, soundness-fatal.
//!
//! V8 fix: detect the overflow boundary and reject with
//! `KernelError::UniverseLevelOverflow`. Honest workloads use
//! single-digit universe levels (typical max is 2 or 3); reaching
//! `u32::MAX` indicates an elaborator bug or an adversarial input.
//!
//! These tests exercise the boundary directly via the public
//! `infer` entry point.

use verum_common::{Heap, List, Maybe};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, KernelError, UniverseLevel, infer,
};

fn empty_axioms() -> AxiomRegistry {
    AxiomRegistry::new()
}

fn empty_ctx() -> Context {
    Context::new()
}

#[test]
fn universe_concrete_zero_types_concrete_one() {
    // Sanity: the V8 fix preserves the standard typing rule for
    // every level below the overflow boundary.
    let term = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let ctx = empty_ctx();
    let axioms = empty_axioms();
    let ty = infer(&ctx, &term, &axioms).expect("infer must succeed");
    match ty {
        CoreTerm::Universe(UniverseLevel::Concrete(1)) => {}
        other => panic!("expected Universe(Concrete(1)), got {:?}", other),
    }
}

#[test]
fn universe_concrete_max_minus_one_types_concrete_max() {
    // Just below the boundary: Concrete(MAX-1) : Concrete(MAX) is
    // legal, no overflow yet.
    let term = CoreTerm::Universe(UniverseLevel::Concrete(u32::MAX - 1));
    let ctx = empty_ctx();
    let axioms = empty_axioms();
    let ty = infer(&ctx, &term, &axioms).expect("infer must succeed");
    match ty {
        CoreTerm::Universe(UniverseLevel::Concrete(n)) if n == u32::MAX => {}
        other => panic!("expected Universe(Concrete(MAX)), got {:?}", other),
    }
}

#[test]
fn b1_universe_concrete_max_overflow_rejected() {
    // The smoking gun: Concrete(MAX) cannot honestly type
    // Concrete(MAX+1) — pre-V8 returned Concrete(MAX) (type-in-type).
    // V8 returns KernelError::UniverseLevelOverflow.
    let term = CoreTerm::Universe(UniverseLevel::Concrete(u32::MAX));
    let ctx = empty_ctx();
    let axioms = empty_axioms();
    let result = infer(&ctx, &term, &axioms);
    match result {
        Err(KernelError::UniverseLevelOverflow { level }) => {
            assert_eq!(level, u32::MAX);
        }
        other => panic!(
            "expected UniverseLevelOverflow, got {:?}",
            other,
        ),
    }
}

#[test]
fn b1_overflow_diagnostic_mentions_type_in_type() {
    // The error's Display message must mention the soundness
    // concern explicitly so post-mortem audit understands why
    // the kernel rejected.
    let err = KernelError::UniverseLevelOverflow { level: u32::MAX };
    let rendered = format!("{}", err);
    assert!(
        rendered.contains("type-in-type"),
        "diagnostic must mention type-in-type concern: {}",
        rendered,
    );
    assert!(
        rendered.contains("K-Univ"),
        "diagnostic must name the rule: {}",
        rendered,
    );
}

#[test]
fn b1_pi_formation_with_max_universe_rejected() {
    // Pi-formation at the boundary: domain has Concrete(MAX), so
    // its inferred type is Concrete(MAX+1) which overflows.
    // Verifies the overflow propagates through compound rules.
    let domain = CoreTerm::Universe(UniverseLevel::Concrete(u32::MAX));
    let codomain = CoreTerm::Universe(UniverseLevel::Concrete(0));
    let term = CoreTerm::Pi {
        binder: verum_common::Text::from("_"),
        domain: Heap::new(domain),
        codomain: Heap::new(codomain),
    };
    let ctx = empty_ctx();
    let axioms = empty_axioms();
    let result = infer(&ctx, &term, &axioms);
    match result {
        Err(KernelError::UniverseLevelOverflow { level }) => {
            assert_eq!(level, u32::MAX);
        }
        other => panic!(
            "expected overflow propagation through Pi, got {:?}",
            other,
        ),
    }
    // Suppress unused-imports lint when nothing else references these.
    let _ = (List::<Heap<CoreTerm>>::new(), Maybe::<u32>::None);
}

#[test]
fn b1_prop_still_types_concrete_zero() {
    // V8 fix is narrow: only the Concrete(MAX) branch is changed.
    // Prop : Type(0) and Variable / Succ / Max paths are untouched.
    let term = CoreTerm::Universe(UniverseLevel::Prop);
    let ctx = empty_ctx();
    let axioms = empty_axioms();
    let ty = infer(&ctx, &term, &axioms).expect("Prop : Type(0)");
    match ty {
        CoreTerm::Universe(UniverseLevel::Concrete(0)) => {}
        other => panic!("expected Universe(Concrete(0)), got {:?}", other),
    }
}
