//! K-Shape / K-Flat / K-Sharp integration tests for cohesive
//! modalities.
//!
//! Cohesive modalities ∫ ⊣ ♭ ⊣ ♯ (Schreiber DCCT, cohesive HoTT).
//! The kernel admits the type-formers unconditionally; the
//! triple-adjunction laws (η, ε, triangle identities) are framework
//! axioms in `core.math.frameworks.schreiber_dcct`. This file covers
//! the kernel-level mechanics:
//!   • Each modality is a type-level endofunctor: `M(A) : Type_i`
//!     when `A : Type_i`.
//!   • Universe levels are preserved (no implicit ascent).
//!   • Modalities applied to non-types are rejected.
//!   • Substitution / normalisation / free-vars all descend.
//!   • Modal-depth ordinal `m_depth_omega` increments by 1 per
//!     modality application — gates K-Refine-omega the same way
//!     ModalBox / ModalDiamond do.

use verum_common::{Heap, Text};
use verum_kernel::{
    AxiomRegistry, Context, CoreTerm, FrameworkId, KernelError, UniverseLevel,
    definitional_eq, infer, normalize,
};
use verum_kernel::depth::{m_depth_omega, OrdinalDepth};

fn empty() -> (Context, AxiomRegistry) {
    (Context::new(), AxiomRegistry::new())
}

fn type0() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}

fn nat_axiom_ty() -> (CoreTerm, AxiomRegistry) {
    // Register a stand-in `Nat : Type_0` axiom we can use to build
    // expressions that need a typeable term inhabiting a universe.
    let mut reg = AxiomRegistry::new();
    let fw = FrameworkId {
        framework: Text::from("test"),
        citation: Text::from("test"),
    };
    let ty = CoreTerm::Universe(UniverseLevel::Concrete(0));
    reg.register(Text::from("Nat"), ty.clone(), fw.clone()).expect("Nat");
    let nat_ref = CoreTerm::Axiom {
        name: Text::from("Nat"),
        ty: Heap::new(ty),
        framework: fw,
    };
    (nat_ref, reg)
}

// =============================================================================
// K-Shape — type formation
// =============================================================================

#[test]
fn shape_of_type_inhabits_same_universe() {
    let (nat_ref, reg) = nat_axiom_ty();
    let term = CoreTerm::Shape(Heap::new(nat_ref));
    let ty = infer(&Context::new(), &term, &reg).expect("∫A typed");
    assert_eq!(ty, type0());
}

#[test]
fn shape_at_higher_universe_preserves_level() {
    let term = CoreTerm::Shape(Heap::new(CoreTerm::Universe(
        UniverseLevel::Concrete(2),
    )));
    let (_, reg) = empty();
    let ty = infer(&Context::new(), &term, &reg).expect("∫Type_2 typed");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(3)));
}

#[test]
fn shape_of_non_type_is_rejected() {
    // A term that's not in a universe — e.g. a variable bound to `Nat`
    // (so the var has type `Nat`, not `Universe`). ∫(var of type Nat)
    // must reject because Nat isn't itself a universe.
    let (nat_ref, reg) = nat_axiom_ty();
    let ctx = Context::new().extend(Text::from("n"), nat_ref);
    let term = CoreTerm::Shape(Heap::new(CoreTerm::Var(Text::from("n"))));
    let res = infer(&ctx, &term, &reg);
    assert!(matches!(res, Err(KernelError::TypeMismatch { .. })));
}

// =============================================================================
// K-Flat — type formation
// =============================================================================

#[test]
fn flat_of_type_inhabits_same_universe() {
    let (nat_ref, reg) = nat_axiom_ty();
    let term = CoreTerm::Flat(Heap::new(nat_ref));
    let ty = infer(&Context::new(), &term, &reg).expect("♭A typed");
    assert_eq!(ty, type0());
}

#[test]
fn flat_of_non_type_is_rejected() {
    let (nat_ref, reg) = nat_axiom_ty();
    let ctx = Context::new().extend(Text::from("n"), nat_ref);
    let term = CoreTerm::Flat(Heap::new(CoreTerm::Var(Text::from("n"))));
    let res = infer(&ctx, &term, &reg);
    assert!(matches!(res, Err(KernelError::TypeMismatch { .. })));
}

// =============================================================================
// K-Sharp — type formation
// =============================================================================

#[test]
fn sharp_of_type_inhabits_same_universe() {
    let (nat_ref, reg) = nat_axiom_ty();
    let term = CoreTerm::Sharp(Heap::new(nat_ref));
    let ty = infer(&Context::new(), &term, &reg).expect("♯A typed");
    assert_eq!(ty, type0());
}

#[test]
fn sharp_at_universe_2_lifts_to_3() {
    let term = CoreTerm::Sharp(Heap::new(CoreTerm::Universe(
        UniverseLevel::Concrete(2),
    )));
    let (_, reg) = empty();
    let ty = infer(&Context::new(), &term, &reg).expect("♯Type_2 typed");
    assert_eq!(ty, CoreTerm::Universe(UniverseLevel::Concrete(3)));
}

// =============================================================================
// Composition: ♭∫A — type formation chain
// =============================================================================

#[test]
fn composed_modalities_typecheck() {
    // ♭(∫(♯A)) — full triple composition.
    let (nat_ref, reg) = nat_axiom_ty();
    let inner = CoreTerm::Sharp(Heap::new(nat_ref));
    let middle = CoreTerm::Shape(Heap::new(inner));
    let outer = CoreTerm::Flat(Heap::new(middle));
    let ty = infer(&Context::new(), &outer, &reg).expect("♭∫♯Nat typed");
    assert_eq!(ty, type0());
}

// =============================================================================
// Modal depth: m_depth_omega increments by 1
// =============================================================================

#[test]
fn modal_depth_omega_shape_increments_by_one() {
    let inner = CoreTerm::Var(Text::from("x"));
    let inner_depth = m_depth_omega(&inner);
    let shaped = CoreTerm::Shape(Heap::new(inner));
    let shaped_depth = m_depth_omega(&shaped);
    assert_eq!(shaped_depth, inner_depth.succ());
}

#[test]
fn modal_depth_omega_flat_increments_by_one() {
    let inner = CoreTerm::Var(Text::from("y"));
    let flatted = CoreTerm::Flat(Heap::new(inner));
    assert_eq!(m_depth_omega(&flatted), OrdinalDepth::finite(1));
}

#[test]
fn modal_depth_omega_sharp_increments_by_one() {
    let inner = CoreTerm::Var(Text::from("z"));
    let sharped = CoreTerm::Sharp(Heap::new(inner));
    assert_eq!(m_depth_omega(&sharped), OrdinalDepth::finite(1));
}

#[test]
fn nested_modalities_accumulate_depth() {
    // ♭(∫(♯ x)) — three modalities → depth 3.
    let term = CoreTerm::Flat(Heap::new(CoreTerm::Shape(Heap::new(
        CoreTerm::Sharp(Heap::new(CoreTerm::Var(Text::from("x")))),
    ))));
    assert_eq!(m_depth_omega(&term), OrdinalDepth::finite(3));
}

// =============================================================================
// Normalisation — structural descent (no β reductions for V1)
// =============================================================================

#[test]
fn shape_normalises_subterm_structurally() {
    // ∫((λx.x) Type_0) ↦ ∫Type_0 by β at the operand.
    let id_app = CoreTerm::App(
        Heap::new(CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(1))),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        }),
        Heap::new(type0()),
    );
    let shaped = CoreTerm::Shape(Heap::new(id_app));
    let normalised = normalize(&shaped);
    assert_eq!(normalised, CoreTerm::Shape(Heap::new(type0())));
}

#[test]
fn definitional_eq_descends_through_modalities() {
    // ∫((λx.x) Type_0) ≡ ∫Type_0.
    let id_app = CoreTerm::App(
        Heap::new(CoreTerm::Lam {
            binder: Text::from("x"),
            domain: Heap::new(CoreTerm::Universe(UniverseLevel::Concrete(1))),
            body: Heap::new(CoreTerm::Var(Text::from("x"))),
        }),
        Heap::new(type0()),
    );
    let lhs = CoreTerm::Shape(Heap::new(id_app));
    let rhs = CoreTerm::Shape(Heap::new(type0()));
    assert!(definitional_eq(&lhs, &rhs));
}

// =============================================================================
// Triple-adjunction laws are framework-axiomatic (kernel does NOT internalise)
// =============================================================================

#[test]
fn kernel_does_not_internally_collapse_flat_shape() {
    // ♭∫A is **not** definitionally equal to A in the kernel —
    // the unit/counit reductions are framework axioms, not
    // built-in. This test pins down the design boundary so the
    // kernel doesn't accidentally bake in a specific cohesive
    // model (cubical vs. simplicial vs. orbispace).
    let (nat_ref, _) = nat_axiom_ty();
    let lhs = CoreTerm::Flat(Heap::new(CoreTerm::Shape(Heap::new(
        nat_ref.clone(),
    ))));
    let rhs = nat_ref;
    assert!(!definitional_eq(&lhs, &rhs));
}
