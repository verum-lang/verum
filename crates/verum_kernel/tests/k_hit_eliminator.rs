//! K-HIT-Form / eliminator auto-generation integration tests
//! (V8 #237, VVA §7.4 + §17.2 Task C3).
//!
//! Higher inductive types extend ordinary inductives with **path
//! constructors** — 1-cells whose endpoints are values of the type
//! itself. The kernel auto-derives the eliminator's type from the
//! registered declaration; this file covers:
//!   • Ordinary inductive recursor shape (Bool, Nat).
//!   • S¹ HIT (one nullary point + one closed-loop path constructor).
//!   • Interval HIT (two nullary points + one path constructor).
//!   • Path-constructor namespace check (collision with point ctor).
//!   • Path-constructor uniqueness check (duplicate path names).
//!   • Path-ctor declarations preserve back-compat with V8 inductive
//!     registration when no path ctors are present.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    ConstructorSig, CoreTerm, InductiveRegistry, KernelError, PathCtorSig,
    RegisteredInductive, UniverseLevel, eliminator_type,
    point_constructor_case_type,
};

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig {
        name: Text::from(name),
        arg_types: List::new(),
    }
}

// =============================================================================
// Ordinary inductive recursor shape — Bool, Nat
// =============================================================================

#[test]
fn bool_eliminator_type_has_motive_two_cases_and_scrutinee_pi() {
    let bool_ind = RegisteredInductive::new(
        Text::from("Bool"),
        List::new(),
        List::from_iter(vec![nullary("True"), nullary("False")]),
    );
    let elim = eliminator_type(&bool_ind);

    // Π (motive : Bool → Type) . Π (case_True : motive(True)) .
    //   Π (case_False : motive(False)) . Π (x : Bool) . motive(x)
    let CoreTerm::Pi {
        binder: motive_b,
        domain: motive_dom,
        codomain: after_motive,
    } = elim
    else {
        panic!("expected outermost Π for motive");
    };
    assert_eq!(motive_b.as_str(), "motive");
    // motive's domain: Π (_ : Bool) . Type_0
    let CoreTerm::Pi { domain: bool_dom, codomain: type_codom, .. } = motive_dom.as_ref()
    else {
        panic!("motive domain must be a Π");
    };
    assert!(matches!(
        bool_dom.as_ref(),
        CoreTerm::Inductive { path, .. } if path.as_str() == "Bool"
    ));
    assert!(matches!(
        type_codom.as_ref(),
        CoreTerm::Universe(UniverseLevel::Concrete(0))
    ));

    // First branch: Π (case_True : motive(True)) . ...
    let CoreTerm::Pi {
        binder: case_true_b,
        domain: case_true_dom,
        codomain: after_true,
    } = after_motive.as_ref()
    else {
        panic!("expected case_True Π");
    };
    assert_eq!(case_true_b.as_str(), "case_True");
    assert!(matches!(
        case_true_dom.as_ref(),
        CoreTerm::App(f, a) if matches!(f.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive")
            && matches!(a.as_ref(), CoreTerm::Var(n) if n.as_str() == "True")
    ));

    // Second branch: case_False.
    let CoreTerm::Pi {
        binder: case_false_b,
        domain: _,
        codomain: after_false,
    } = after_true.as_ref()
    else {
        panic!("expected case_False Π");
    };
    assert_eq!(case_false_b.as_str(), "case_False");

    // Innermost: Π (x : Bool) . motive(x).
    let CoreTerm::Pi { binder: x_b, codomain: ret, .. } = after_false.as_ref()
    else {
        panic!("expected scrutinee Π");
    };
    assert_eq!(x_b.as_str(), "x");
    assert!(matches!(
        ret.as_ref(),
        CoreTerm::App(f, a) if matches!(f.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive")
            && matches!(a.as_ref(), CoreTerm::Var(n) if n.as_str() == "x")
    ));
}

#[test]
fn nat_succ_case_takes_pi_argument() {
    // Nat = Zero | Succ(Nat)
    let nat_ind = RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![
            nullary("Zero"),
            ConstructorSig {
                name: Text::from("Succ"),
                arg_types: List::from_iter(vec![CoreTerm::Inductive {
                    path: Text::from("Nat"),
                    args: List::new(),
                }]),
            },
        ]),
    );
    let elim = eliminator_type(&nat_ind);
    // Skip motive + Zero case to reach the Succ case.
    let CoreTerm::Pi { codomain: after_motive, .. } = elim else {
        panic!()
    };
    let CoreTerm::Pi { codomain: after_zero, .. } = after_motive.as_ref() else {
        panic!()
    };
    let CoreTerm::Pi {
        binder: case_succ_b,
        domain: succ_dom,
        ..
    } = after_zero.as_ref()
    else {
        panic!()
    };
    assert_eq!(case_succ_b.as_str(), "case_Succ");
    // succ_dom: Π (a0 : Nat) . motive(Succ(a0))
    let CoreTerm::Pi {
        binder: a0,
        domain: a0_ty,
        codomain: succ_goal,
    } = succ_dom.as_ref()
    else {
        panic!("Succ case must be a Π over its argument");
    };
    assert_eq!(a0.as_str(), "a0");
    assert!(matches!(
        a0_ty.as_ref(),
        CoreTerm::Inductive { path, .. } if path.as_str() == "Nat"
    ));
    // goal: motive(Succ(a0))
    let CoreTerm::App(motive, succ_app) = succ_goal.as_ref() else {
        panic!()
    };
    assert!(matches!(motive.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive"));
    let CoreTerm::App(succ_var, a0_var) = succ_app.as_ref() else {
        panic!()
    };
    assert!(matches!(succ_var.as_ref(), CoreTerm::Var(n) if n.as_str() == "Succ"));
    assert!(matches!(a0_var.as_ref(), CoreTerm::Var(n) if n.as_str() == "a0"));
}

// =============================================================================
// HIT recursor — S¹ + Interval
// =============================================================================

#[test]
fn s1_eliminator_has_path_branch() {
    // S¹ = Base | Loop : Base..Base
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"),
        lhs: CoreTerm::Var(Text::from("Base")),
        rhs: CoreTerm::Var(Text::from("Base")),
    });
    let elim = eliminator_type(&s1);
    // V8 (#237 V2) recursor-image resolution:
    //   Π (motive). Π (case_Base : motive(Base)).
    //     Π (case_Loop : PathTy(motive(Base), case_Base, case_Base)).
    //     Π (x : S1). motive(x)
    let CoreTerm::Pi { codomain: after_motive, .. } = elim else {
        panic!()
    };
    let CoreTerm::Pi { codomain: after_base, .. } = after_motive.as_ref() else {
        panic!()
    };
    let CoreTerm::Pi {
        binder: case_loop_b,
        domain: loop_dom,
        codomain: after_loop,
    } = after_base.as_ref()
    else {
        panic!()
    };
    assert_eq!(case_loop_b.as_str(), "case_Loop");
    let CoreTerm::PathTy { carrier, lhs, rhs } = loop_dom.as_ref() else {
        panic!("Loop branch must be a PathTy");
    };
    // carrier = motive(Base)
    assert!(matches!(
        carrier.as_ref(),
        CoreTerm::App(f, a)
            if matches!(f.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive")
                && matches!(a.as_ref(), CoreTerm::Var(n) if n.as_str() == "Base")
    ));
    // V2: lhs = rhs = case_Base (recursor's image at the closed loop).
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Base"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Base"));
    // Innermost remains Π (x : S1) . motive(x).
    let CoreTerm::Pi { binder, .. } = after_loop.as_ref() else { panic!() };
    assert_eq!(binder.as_str(), "x");
}

#[test]
fn interval_eliminator_has_two_points_and_seg_branch() {
    // Interval = Zero | One | Seg : Zero..One
    let interval = RegisteredInductive::new(
        Text::from("Interval"),
        List::new(),
        List::from_iter(vec![nullary("Zero"), nullary("One")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Seg"),
        lhs: CoreTerm::Var(Text::from("Zero")),
        rhs: CoreTerm::Var(Text::from("One")),
    });
    let elim = eliminator_type(&interval);
    // Walk past motive + Zero + One.
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    // Now the Seg branch.
    let CoreTerm::Pi { binder, domain, .. } = a.as_ref() else { panic!() };
    assert_eq!(binder.as_str(), "case_Seg");
    let CoreTerm::PathTy { lhs, rhs, .. } = domain.as_ref() else {
        panic!("Seg branch must be a PathTy");
    };
    // V8 (#237 V2): nullary endpoints rewrite to recursor-image
    // references so the Seg branch types against the user-supplied
    // case bodies, not the bare ctor values.
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Zero"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_One"));
}

// =============================================================================
// Registration namespace + uniqueness validation
// =============================================================================

#[test]
fn register_rejects_path_ctor_colliding_with_point_ctor() {
    let mut reg = InductiveRegistry::new();
    let bad = RegisteredInductive::new(
        Text::from("Bad"),
        List::new(),
        List::from_iter(vec![nullary("X")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("X"), // collides with point ctor "X"
        lhs: CoreTerm::Var(Text::from("X")),
        rhs: CoreTerm::Var(Text::from("X")),
    });
    let result = reg.register(bad);
    assert!(matches!(result, Err(KernelError::DuplicateInductive(_))));
}

#[test]
fn register_rejects_duplicate_path_ctor_names() {
    let mut reg = InductiveRegistry::new();
    let bad = RegisteredInductive::new(
        Text::from("BadHit"),
        List::new(),
        List::from_iter(vec![nullary("Pt")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"),
        lhs: CoreTerm::Var(Text::from("Pt")),
        rhs: CoreTerm::Var(Text::from("Pt")),
    })
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"), // duplicate
        lhs: CoreTerm::Var(Text::from("Pt")),
        rhs: CoreTerm::Var(Text::from("Pt")),
    });
    let result = reg.register(bad);
    assert!(matches!(result, Err(KernelError::DuplicateInductive(_))));
}

#[test]
fn register_admits_back_compat_inductives_without_path_ctors() {
    // Pre-V8 declarations omit `path_constructors` entirely; the
    // serde-default empty List preserves their semantics.
    let mut reg = InductiveRegistry::new();
    let nat = RegisteredInductive::new(
        Text::from("BackCompatNat"),
        List::new(),
        List::from_iter(vec![nullary("Z")]),
    );
    assert!(reg.register(nat).is_ok());
}

// =============================================================================
// point_constructor_case_type direct
// =============================================================================

#[test]
fn nullary_ctor_case_type_is_motive_app() {
    let motive = CoreTerm::Var(Text::from("motive"));
    let ctor = nullary("Leaf");
    let ty = point_constructor_case_type(&motive, &ctor);
    assert!(matches!(
        ty,
        CoreTerm::App(f, a)
            if matches!(f.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive")
                && matches!(a.as_ref(), CoreTerm::Var(n) if n.as_str() == "Leaf")
    ));
}

#[test]
fn binary_ctor_case_type_chains_pi() {
    let motive = CoreTerm::Var(Text::from("motive"));
    // Pair : (Int, Int) → Pair
    let int_ty = CoreTerm::Inductive {
        path: Text::from("Int"),
        args: List::new(),
    };
    let ctor = ConstructorSig {
        name: Text::from("Pair"),
        arg_types: List::from_iter(vec![int_ty.clone(), int_ty]),
    };
    let ty = point_constructor_case_type(&motive, &ctor);
    // Π (a0 : Int) . Π (a1 : Int) . motive(Pair(a0, a1))
    let CoreTerm::Pi { binder, codomain, .. } = ty else { panic!() };
    assert_eq!(binder.as_str(), "a0");
    let CoreTerm::Pi { binder, codomain, .. } = codomain.as_ref() else {
        panic!()
    };
    assert_eq!(binder.as_str(), "a1");
    // Innermost: motive(App(App(Pair, a0), a1)).
    let CoreTerm::App(motive_ref, pair_app) = codomain.as_ref() else {
        panic!()
    };
    assert!(matches!(motive_ref.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive"));
    let CoreTerm::App(pair_a0, a1) = pair_app.as_ref() else { panic!() };
    let CoreTerm::App(pair, a0) = pair_a0.as_ref() else { panic!() };
    assert!(matches!(pair.as_ref(), CoreTerm::Var(n) if n.as_str() == "Pair"));
    assert!(matches!(a0.as_ref(), CoreTerm::Var(n) if n.as_str() == "a0"));
    assert!(matches!(a1.as_ref(), CoreTerm::Var(n) if n.as_str() == "a1"));
}

// =============================================================================
// Universe-level preservation
// =============================================================================

// =============================================================================
// V8 (#237 V2) — recursor-image resolution at nullary endpoints
// =============================================================================

#[test]
fn nullary_endpoint_rewrites_to_case_binder() {
    // S¹ → case_Loop : PathTy(motive(Base), case_Base, case_Base).
    // Without V2 resolution the endpoints would be `Var("Base")` —
    // wrong shape for the recursor's image, which lives at
    // `motive(Base)`.
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"),
        lhs: CoreTerm::Var(Text::from("Base")),
        rhs: CoreTerm::Var(Text::from("Base")),
    });
    let elim = eliminator_type(&s1);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    let CoreTerm::PathTy { lhs, rhs, .. } = domain.as_ref() else { panic!() };
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Base"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Base"));
}

#[test]
fn endpoint_not_referencing_point_ctor_falls_through_unchanged() {
    // If an endpoint references something OTHER than a registered
    // point ctor (e.g. an external constant `External`), V2 must
    // leave it alone — only point-ctor names rewrite to case-binders.
    let hit = RegisteredInductive::new(
        Text::from("Weird"),
        List::new(),
        List::from_iter(vec![nullary("Pt")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Edge"),
        lhs: CoreTerm::Var(Text::from("External")),
        rhs: CoreTerm::Var(Text::from("External")),
    });
    let elim = eliminator_type(&hit);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    let CoreTerm::PathTy { lhs, rhs, .. } = domain.as_ref() else { panic!() };
    // External is NOT a registered point ctor, so it stays as-is.
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "External"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "External"));
}

#[test]
fn app_chain_endpoint_falls_through_unchanged() {
    // V2 only resolves bare-Var endpoints. A non-nullary endpoint
    // like `App(Var("Cons"), Var("a0"))` falls through to the
    // elaborator (V3 follow-up).
    let hit = RegisteredInductive::new(
        Text::from("Sus"),
        List::new(),
        List::from_iter(vec![nullary("North"), nullary("South")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Merid"),
        lhs: CoreTerm::App(
            verum_common::Heap::new(CoreTerm::Var(Text::from("North"))),
            verum_common::Heap::new(CoreTerm::Var(Text::from("a0"))),
        ),
        rhs: CoreTerm::Var(Text::from("South")),
    });
    let elim = eliminator_type(&hit);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    let CoreTerm::PathTy { lhs, rhs, .. } = domain.as_ref() else { panic!() };
    // App-chain lhs unchanged — V2 only resolves bare Var.
    assert!(matches!(lhs.as_ref(), CoreTerm::App(..)));
    // Bare Var rhs ("South") IS a point ctor → resolves to case_South.
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_South"));
}

#[test]
fn elim_preserves_declared_universe_level() {
    let hit = RegisteredInductive::new(
        Text::from("HighLevel"),
        List::new(),
        List::from_iter(vec![nullary("P")]),
    )
    .with_universe(UniverseLevel::Concrete(2));
    let elim = eliminator_type(&hit);
    // motive : T → Type_2.
    let CoreTerm::Pi { domain: motive_dom, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: type_codom, .. } = motive_dom.as_ref() else {
        panic!()
    };
    assert_eq!(
        type_codom.as_ref(),
        &CoreTerm::Universe(UniverseLevel::Concrete(2))
    );
    let _ = Heap::new(()); // silence unused-import if any
}
