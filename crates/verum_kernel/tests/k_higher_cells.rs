//! n-cell HIT eliminator tests.
//!
//! Higher inductive types extend 1-cells (paths) to 2-cells
//! (homotopies between paths), 3-cells (homotopies between
//! homotopies), …, n-cells. Per the kernel surface
//! supports arbitrary `dim` via [`PathCtorSig::n_cell`]; the
//! eliminator emit recursively nests `PathOver` `dim` times so
//! the per-dimensional shape is recorded.
//!
//! Coverage:
//!   • 1-cell (dim=1) — homogeneous (S¹) and heterogeneous (Interval)
//!     cases; baseline check that nothing regresses vs the existing
//!     dim=1 emit.
//!   • 2-cell (dim=2) — Torus-style surface 2-cell whose endpoints
//!     are themselves 1-cells (path expressions). Verifies the
//!     eliminator branch is wrapped in PathOver-of-PathOver shape.
//!   • 3-cell (dim=3) — synthetic cell to exercise the recursive
//!     nesting beyond two levels.
//!   • Default dim — `PathCtorSig::one_cell` constructs dim=1.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    ConstructorSig, CoreTerm, PathCtorSig, RegisteredInductive,
    eliminator_type,
};

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig { name: Text::from(name), arg_types: List::new() }
}

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

#[test]
fn path_ctor_one_cell_helper_constructs_dim_1() {
    let sig = PathCtorSig::one_cell(
        Text::from("Loop"),
        var("Base"),
        var("Base"),
    );
    assert_eq!(sig.dim, 1);
}

#[test]
fn path_ctor_n_cell_helper_constructs_with_explicit_dim() {
    let sig = PathCtorSig::n_cell(
        Text::from("Surf"),
        2,
        var("loop_a"),
        var("loop_b"),
    );
    assert_eq!(sig.dim, 2);
}

// =============================================================================
// 2-cell HIT eliminator emit
// =============================================================================

#[test]
fn two_cell_eliminator_branch_nests_pathover() {
    // Torus-shape HIT: Base | Surf : (loop_a · loop_b) ↝ (loop_b · loop_a)
    // dim=2 — eliminator's case_Surf branch must be PathOver wrapping
    // PathOver (one extra layer above the dim=1 case).
    let torus = RegisteredInductive::new(
        Text::from("Torus"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig::n_cell(
        Text::from("Surf"),
        2,
        var("loop_a"),
        var("loop_b"),
    ));
    let elim = eliminator_type(&torus);
    // Walk past motive + case_Base.
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { binder, domain, .. } = a.as_ref() else { panic!() };
    assert_eq!(binder.as_str(), "case_Surf");
    // dim=2 ⇒ outer PathOver wrapping inner PathOver.
    let CoreTerm::PathOver { lhs: outer_lhs, path: outer_path, .. } =
        domain.as_ref()
    else {
        panic!("dim=2 branch must be PathOver-shaped at outer layer; got {:?}", domain.as_ref());
    };
    // Inner layer (the dim=1 PathOver) lives in outer_lhs.
    let inner = outer_lhs.as_ref();
    assert!(
        matches!(inner, CoreTerm::PathOver { .. }),
        "outer PathOver's lhs must wrap inner PathOver (dim=2 → 2 nested PathOvers); got {:?}",
        inner
    );
    // Outer path slot must reify the n-cell as a PathTy chain.
    assert!(
        matches!(outer_path.as_ref(), CoreTerm::PathTy { .. }),
        "outer path slot must be a PathTy reifying the cell shape; got {:?}",
        outer_path.as_ref()
    );
}

#[test]
fn three_cell_eliminator_branch_nests_pathover_thrice() {
    // Synthetic 3-cell HIT — verify recursion handles arbitrary depth.
    let three = RegisteredInductive::new(
        Text::from("ThreeCell"),
        List::new(),
        List::from_iter(vec![nullary("P")]),
    )
    .with_path_constructor(PathCtorSig::n_cell(
        Text::from("Cell3"),
        3,
        var("two_cell_a"),
        var("two_cell_b"),
    ));
    let elim = eliminator_type(&three);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { binder, domain, .. } = a.as_ref() else { panic!() };
    assert_eq!(binder.as_str(), "case_Cell3");

    // Count PathOver nesting depth through the lhs spine.
    fn count_pathover_lhs_depth(t: &CoreTerm) -> u32 {
        match t {
            CoreTerm::PathOver { lhs, .. } => 1 + count_pathover_lhs_depth(lhs),
            _ => 0,
        }
    }
    let depth = count_pathover_lhs_depth(domain.as_ref());
    assert_eq!(
        depth, 3,
        "dim=3 must yield 3 nested PathOver layers in the lhs spine; got {}",
        depth
    );
}

#[test]
fn one_cell_eliminator_unchanged_after_dim_field_landing() {
    // Regression: dim=1 path constructor must produce the same
    // single-PathOver / single-PathTy shape as before .
    let interval = RegisteredInductive::new(
        Text::from("Interval"),
        List::new(),
        List::from_iter(vec![nullary("Zero"), nullary("One")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("Seg"),
        var("Zero"),
        var("One"),
    ));
    let elim = eliminator_type(&interval);
    let CoreTerm::Pi { codomain: a, .. } = elim else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { codomain: a, .. } = a.as_ref() else { panic!() };
    let CoreTerm::Pi { domain, .. } = a.as_ref() else { panic!() };
    // Single layer (PathOver because heterogeneous).
    let CoreTerm::PathOver { lhs, .. } = domain.as_ref() else {
        panic!("dim=1 heterogeneous Seg branch must be a single PathOver");
    };
    // lhs is NOT another PathOver (no double nesting at dim=1).
    assert!(
        !matches!(lhs.as_ref(), CoreTerm::PathOver { .. }),
        "dim=1 must produce single-layer PathOver, not nested; lhs={:?}",
        lhs.as_ref()
    );
}
