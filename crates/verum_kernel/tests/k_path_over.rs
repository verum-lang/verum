//! PathOver kernel-level tests.
//!
//! `PathOver(motive, p, lhs, rhs)` is the dependent path-over
//! constructor needed when a HIT path-constructor's endpoints have
//! distinct motive images. This file pins:
//!
//!   • PathOver typing rule (K-PathOver-Form): motive must be a Pi
//!     B → U, path must be a PathTy, endpoints must type-check;
//!     result inhabits the same universe as motive's codomain.
//!   • Degenerate-case reduction: PathOver(motive, p, lhs, rhs)
//!     where `p`'s endpoints coincide structurally collapses to
//!     homogeneous PathTy(motive(b₀), lhs, rhs).
//!   • Substitution + free-vars compatibility under PathOver.
//!   • Eliminator integration: heterogeneous-endpoint HITs (Susp,
//!     Interval) emit PathOver branches; closed-loop HITs (S¹)
//!     emit PathTy branches.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    AxiomRegistry, ConstructorSig, Context, CoreTerm, InductiveRegistry,
    PathCtorSig, RegisteredInductive, UniverseLevel, eliminator_type, infer,
    normalize, normalize_with_inductives, support::{free_vars, substitute},
};

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn type0() -> CoreTerm {
    CoreTerm::Universe(UniverseLevel::Concrete(0))
}

// =============================================================================
// K-PathOver-Form typing rule
// =============================================================================

#[test]
fn pathover_typing_admits_well_formed() {
    // motive : Nat → Type_0
    let motive = CoreTerm::Lam {
        binder: Text::from("n"),
        domain: Heap::new(CoreTerm::Inductive {
            path: Text::from("Nat"),
            args: List::new(),
        }),
        body: Heap::new(type0()),
    };
    // path = Refl(Zero) which has type Path<Nat>(Zero, Zero).
    let path = CoreTerm::Refl(Heap::new(var("Zero")));
    let lhs = type0();
    let rhs = type0();
    let term = CoreTerm::PathOver {
        motive: Heap::new(motive),
        path: Heap::new(path),
        lhs: Heap::new(lhs),
        rhs: Heap::new(rhs),
    };
    // Need Nat in the inductive registry so the path's carrier types.
    let mut inds = InductiveRegistry::new();
    inds.register(RegisteredInductive::new(
        Text::from("Nat"),
        List::new(),
        List::from_iter(vec![
            ConstructorSig { name: Text::from("Zero"), arg_types: List::new() },
        ]),
    )).unwrap();
    // Need Zero as a variable in context (we don't model ctor as
    // first-class Var typing; tests use a ctx with Zero : Nat).
    let ctx = Context::new().extend(
        Text::from("Zero"),
        CoreTerm::Inductive { path: Text::from("Nat"), args: List::new() },
    );
    let axioms = AxiomRegistry::new();
    let ty = infer(&ctx, &term, &axioms).expect("PathOver must type-check");
    // Motive Lam(_:Nat, Type_0) has type Pi(Nat, Type_1) — its
    // codomain is `Type_0` whose own type is `Type_1`. So the
    // PathOver inhabits `Type_1`, the codomain's universe level.
    assert!(
        matches!(ty, CoreTerm::Universe(_)),
        "PathOver must inhabit a universe; got {:?}",
        ty
    );
}

#[test]
fn pathover_typing_rejects_non_pi_motive() {
    // motive is just `type0()` — not a Pi. Must reject.
    let term = CoreTerm::PathOver {
        motive: Heap::new(type0()),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(type0()),
            lhs: Heap::new(var("a")),
            rhs: Heap::new(var("b")),
        }),
        lhs: Heap::new(var("x")),
        rhs: Heap::new(var("y")),
    };
    let ctx = Context::new()
        .extend(Text::from("a"), type0())
        .extend(Text::from("b"), type0())
        .extend(Text::from("x"), type0())
        .extend(Text::from("y"), type0());
    let axioms = AxiomRegistry::new();
    let result = infer(&ctx, &term, &axioms);
    assert!(
        result.is_err(),
        "PathOver with non-Pi motive must reject; got {:?}",
        result
    );
}

#[test]
fn pathover_typing_admits_arbitrary_path_shape_in_v3_0() {
    // V3.0 weak typing rule: path slot accepts (a) a term of type
    // PathTy (proper case) or (b) a PathTy term used as path-shape
    // annotation by the HIT eliminator emitter. The strict "path
    // is a PathTy-typed term" check is V3.1 follow-up.
    let motive = CoreTerm::Lam {
        binder: Text::from("_"),
        domain: Heap::new(type0()),
        body: Heap::new(type0()),
    };
    let term = CoreTerm::PathOver {
        motive: Heap::new(motive),
        path: Heap::new(var("any_term")),
        lhs: Heap::new(var("x")),
        rhs: Heap::new(var("y")),
    };
    let ctx = Context::new()
        .extend(Text::from("any_term"), type0())
        .extend(Text::from("x"), type0())
        .extend(Text::from("y"), type0());
    let axioms = AxiomRegistry::new();
    let ty = infer(&ctx, &term, &axioms).expect("V3.0 admits any well-typed path slot");
    assert!(matches!(ty, CoreTerm::Universe(_)));
}

// =============================================================================
// Degenerate-case reduction: closed-loop path → PathTy collapse
// =============================================================================

#[test]
fn pathover_normalize_collapses_closed_loop_to_pathty() {
    // PathOver(motive, PathTy(B, base, base), lhs, rhs)
    //   ↦ PathTy(motive(base), lhs, rhs)
    let motive = var("M");
    let term = CoreTerm::PathOver {
        motive: Heap::new(motive.clone()),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("B")),
            lhs: Heap::new(var("base")),
            rhs: Heap::new(var("base")),
        }),
        lhs: Heap::new(var("lhs_image")),
        rhs: Heap::new(var("rhs_image")),
    };
    let normal = normalize(&term);
    let CoreTerm::PathTy { carrier, lhs, rhs } = &normal else {
        panic!("closed-loop PathOver must collapse to PathTy; got {:?}", normal);
    };
    // carrier = M(base)
    let CoreTerm::App(f, a) = carrier.as_ref() else {
        panic!("carrier must be M(base) App, got {:?}", carrier.as_ref());
    };
    assert!(matches!(f.as_ref(), CoreTerm::Var(n) if n.as_str() == "M"));
    assert!(matches!(a.as_ref(), CoreTerm::Var(n) if n.as_str() == "base"));
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "lhs_image"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "rhs_image"));
}

#[test]
fn pathover_normalize_keeps_heterogeneous_pathover() {
    // Endpoints differ (Zero ≠ One) — must keep PathOver shape.
    let term = CoreTerm::PathOver {
        motive: Heap::new(var("M")),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("B")),
            lhs: Heap::new(var("Zero")),
            rhs: Heap::new(var("One")),
        }),
        lhs: Heap::new(var("lhs_image")),
        rhs: Heap::new(var("rhs_image")),
    };
    let normal = normalize(&term);
    assert!(
        matches!(normal, CoreTerm::PathOver { .. }),
        "heterogeneous PathOver must NOT collapse; got {:?}",
        normal
    );
}

#[test]
fn pathover_collapse_works_under_inductive_aware_normaliser() {
    let inds = InductiveRegistry::new();
    let term = CoreTerm::PathOver {
        motive: Heap::new(var("M")),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("B")),
            lhs: Heap::new(var("p")),
            rhs: Heap::new(var("p")),
        }),
        lhs: Heap::new(var("a")),
        rhs: Heap::new(var("b")),
    };
    let normal = normalize_with_inductives(&term, &inds);
    assert!(
        matches!(normal, CoreTerm::PathTy { .. }),
        "inductive-aware normaliser must also collapse closed-loop PathOver; got {:?}",
        normal
    );
}

// =============================================================================
// Substitution + free-vars compatibility
// =============================================================================

#[test]
fn pathover_substitute_walks_all_components() {
    // PathOver(M, PathTy(B, x, x), x, x) [x := y]
    //   = PathOver(M, PathTy(B, y, y), y, y)
    let term = CoreTerm::PathOver {
        motive: Heap::new(var("M")),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("B")),
            lhs: Heap::new(var("x")),
            rhs: Heap::new(var("x")),
        }),
        lhs: Heap::new(var("x")),
        rhs: Heap::new(var("x")),
    };
    let subbed = substitute(&term, "x", &var("y"));
    let CoreTerm::PathOver { path, lhs, rhs, .. } = &subbed else {
        panic!("substitution must preserve PathOver shape");
    };
    let CoreTerm::PathTy { lhs: pl, rhs: pr, .. } = path.as_ref() else {
        panic!()
    };
    assert!(matches!(pl.as_ref(), CoreTerm::Var(n) if n.as_str() == "y"));
    assert!(matches!(pr.as_ref(), CoreTerm::Var(n) if n.as_str() == "y"));
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "y"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "y"));
}

#[test]
fn pathover_free_vars_collects_all_components() {
    let term = CoreTerm::PathOver {
        motive: Heap::new(var("M")),
        path: Heap::new(CoreTerm::PathTy {
            carrier: Heap::new(var("B")),
            lhs: Heap::new(var("p_lhs")),
            rhs: Heap::new(var("p_rhs")),
        }),
        lhs: Heap::new(var("ov_lhs")),
        rhs: Heap::new(var("ov_rhs")),
    };
    let fvs = free_vars(&term);
    let names: Vec<&str> = fvs.iter().map(|t| t.as_str()).collect();
    assert!(names.contains(&"M"));
    assert!(names.contains(&"B"));
    assert!(names.contains(&"p_lhs"));
    assert!(names.contains(&"p_rhs"));
    assert!(names.contains(&"ov_lhs"));
    assert!(names.contains(&"ov_rhs"));
}

// =============================================================================
// Eliminator integration: PathOver vs PathTy emit
// =============================================================================

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig {
        name: Text::from(name),
        arg_types: List::new(),
    }
}

#[test]
fn s1_eliminator_uses_pathty_for_closed_loop() {
    // S¹: Base | Loop : Base ↝ Base — closed loop, homogeneous.
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Loop"),
        dim: 1,
        lhs: var("Base"),
        rhs: var("Base"),
    });
    let elim = eliminator_type(&s1);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    assert!(
        matches!(domain.as_ref(), CoreTerm::PathTy { .. }),
        "S¹ closed-loop branch must use homogeneous PathTy, not PathOver; got {:?}",
        domain.as_ref()
    );
}

#[test]
fn interval_eliminator_uses_pathover_for_distinct_endpoints() {
    // Interval: Zero | One | Seg : Zero ↝ One — heterogeneous.
    let interval = RegisteredInductive::new(
        Text::from("Interval"),
        List::new(),
        List::from_iter(vec![nullary("Zero"), nullary("One")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Seg"),
        dim: 1,
        lhs: var("Zero"),
        rhs: var("One"),
    });
    let elim = eliminator_type(&interval);
    // Walk past motive + Zero + One.
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    assert!(
        matches!(domain.as_ref(), CoreTerm::PathOver { .. }),
        "Interval Seg branch must use dependent PathOver, not homogeneous PathTy; got {:?}",
        domain.as_ref()
    );
}

#[test]
fn pathover_branch_carries_motive_and_constructor_path() {
    // Same Interval HIT — verify the PathOver branch carries the
    // right motive + path components.
    let interval = RegisteredInductive::new(
        Text::from("Interval"),
        List::new(),
        List::from_iter(vec![nullary("Zero"), nullary("One")]),
    )
    .with_path_constructor(PathCtorSig {
        name: Text::from("Seg"),
        dim: 1,
        lhs: var("Zero"),
        rhs: var("One"),
    });
    let elim = eliminator_type(&interval);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    let CoreTerm::PathOver { motive, path, lhs, rhs } = domain.as_ref() else {
        panic!("expected PathOver branch")
    };
    // motive is the bound `motive` Var.
    assert!(matches!(motive.as_ref(), CoreTerm::Var(n) if n.as_str() == "motive"));
    // path is PathTy(Inductive(Interval), Zero, One).
    let CoreTerm::PathTy { carrier, lhs: pl, rhs: pr } = path.as_ref() else {
        panic!("constructor-path must be reified as PathTy")
    };
    assert!(matches!(
        carrier.as_ref(),
        CoreTerm::Inductive { path, .. } if path.as_str() == "Interval"
    ));
    assert!(matches!(pl.as_ref(), CoreTerm::Var(n) if n.as_str() == "Zero"));
    assert!(matches!(pr.as_ref(), CoreTerm::Var(n) if n.as_str() == "One"));
    // Recursor-image endpoints (V2 nullary resolution).
    assert!(matches!(lhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_Zero"));
    assert!(matches!(rhs.as_ref(), CoreTerm::Var(n) if n.as_str() == "case_One"));
}
