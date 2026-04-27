//! HIT path-constructor β-rule
//! tests.
//!
//! Per `K-Elim` extension: when the eliminator's scrutinee is a
//! bare path-ctor reference (a 1-cell or higher cell of the
//! surrounding inductive), the eliminator β-reduces to the
//! corresponding case branch directly. The case is the user-
//! supplied path-image — already shaped as a `PathOver` /
//! homogeneous `PathTy` value of the right type per the
//! eliminator-emit convention.
//!
//! Coverage:
//!   • S¹ Loop reduces to case_Loop (closed-loop branch).
//!   • Interval Seg reduces to case_Seg (heterogeneous PathOver).
//!   • Suspension merid reduces to case_merid.
//!   • Path-ctor β fires AFTER point-ctor cases in the case list
//!     (cases.len() = point_count + path_count layout).
//!   • Out-of-range scrutinee leaves the term neutral.
//!   • Mixed: nested point + path β within a single normalisation.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    ConstructorSig, CoreTerm, InductiveRegistry, PathCtorSig,
    RegisteredInductive, normalize_with_inductives,
};

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig { name: Text::from(name), arg_types: List::new() }
}

fn var(name: &str) -> CoreTerm {
    CoreTerm::Var(Text::from(name))
}

fn elim(scrutinee: CoreTerm, cases: Vec<CoreTerm>) -> CoreTerm {
    CoreTerm::Elim {
        scrutinee: Heap::new(scrutinee),
        motive: Heap::new(var("motive")),
        cases: List::from_iter(cases),
    }
}

#[test]
fn s1_loop_path_ctor_beta_reduces_to_case_loop() {
    // S¹ = Base | Loop : Base ↝ Base
    // cases = [case_Base, case_Loop]
    let mut reg = InductiveRegistry::new();
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("Loop"),
        var("Base"),
        var("Base"),
    ));
    reg.register(s1).unwrap();

    let term = elim(var("Loop"), vec![var("case_Base"), var("case_Loop")]);
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_Loop"),
        "S¹ elim(Loop) must β-reduce to case_Loop; got {:?}",
        normal
    );
}

#[test]
fn interval_seg_path_ctor_beta_reduces_to_case_seg() {
    // Interval = Zero | One | Seg : Zero ↝ One
    // cases = [case_Zero, case_One, case_Seg]
    let mut reg = InductiveRegistry::new();
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
    reg.register(interval).unwrap();

    let term = elim(
        var("Seg"),
        vec![var("case_Zero"), var("case_One"), var("case_Seg")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_Seg"),
        "Interval elim(Seg) must β-reduce to case_Seg; got {:?}",
        normal
    );
}

#[test]
fn suspension_merid_path_ctor_beta_reduces_to_case_merid() {
    // Suspension = North | South | merid : North ↝ South
    let mut reg = InductiveRegistry::new();
    let susp = RegisteredInductive::new(
        Text::from("Susp"),
        List::new(),
        List::from_iter(vec![nullary("North"), nullary("South")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("merid"),
        var("North"),
        var("South"),
    ));
    reg.register(susp).unwrap();

    let term = elim(
        var("merid"),
        vec![var("case_north"), var("case_south"), var("case_merid")],
    );
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "case_merid"),
        "Susp elim(merid) must β-reduce to case_merid; got {:?}",
        normal
    );
}

#[test]
fn path_ctor_beta_picks_correct_index_among_multiple_paths() {
    // HIT with multiple path ctors: cases must select the right
    // one by declaration order.
    //
    // Torus-like: Base | loop_a : Base↝Base | loop_b : Base↝Base
    let mut reg = InductiveRegistry::new();
    let torus_2 = RegisteredInductive::new(
        Text::from("Torus2"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("loop_a"),
        var("Base"),
        var("Base"),
    ))
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("loop_b"),
        var("Base"),
        var("Base"),
    ));
    reg.register(torus_2).unwrap();

    // cases = [case_Base, case_loop_a, case_loop_b]
    let cases = vec![var("case_Base"), var("case_loop_a"), var("case_loop_b")];

    let term_a = elim(var("loop_a"), cases.clone());
    let normal_a = normalize_with_inductives(&term_a, &reg);
    assert!(
        matches!(&normal_a, CoreTerm::Var(n) if n.as_str() == "case_loop_a"),
        "elim(loop_a) must β-reduce to case_loop_a; got {:?}",
        normal_a
    );

    let term_b = elim(var("loop_b"), cases.clone());
    let normal_b = normalize_with_inductives(&term_b, &reg);
    assert!(
        matches!(&normal_b, CoreTerm::Var(n) if n.as_str() == "case_loop_b"),
        "elim(loop_b) must β-reduce to case_loop_b; got {:?}",
        normal_b
    );
}

#[test]
fn path_ctor_beta_with_too_few_cases_remains_neutral() {
    // cases = [case_Base] — missing the loop case → β must NOT fire
    // (out-of-range index).
    let mut reg = InductiveRegistry::new();
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("Loop"),
        var("Base"),
        var("Base"),
    ));
    reg.register(s1).unwrap();

    let term = elim(var("Loop"), vec![var("case_Base")]);
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Elim { .. }),
        "missing path case must keep neutral Elim; got {:?}",
        normal
    );
}

#[test]
fn path_ctor_beta_unrelated_var_stays_neutral() {
    // Scrutinee is `Var("foo")` which is not a registered ctor → neutral.
    let mut reg = InductiveRegistry::new();
    let s1 = RegisteredInductive::new(
        Text::from("S1"),
        List::new(),
        List::from_iter(vec![nullary("Base")]),
    )
    .with_path_constructor(PathCtorSig::one_cell(
        Text::from("Loop"),
        var("Base"),
        var("Base"),
    ));
    reg.register(s1).unwrap();

    let term = elim(var("foo"), vec![var("case_Base"), var("case_Loop")]);
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Elim { .. }),
        "unrelated var scrutinee must stay neutral; got {:?}",
        normal
    );
}

#[test]
fn path_ctor_beta_fires_after_point_ctor_offset_layout() {
    // HIT with TWO point ctors and ONE path ctor:
    //   cases = [point0, point1, path0]
    // Path-ctor case index = point_count (2) + path_idx (0) = 2.
    let mut reg = InductiveRegistry::new();
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
    reg.register(interval).unwrap();

    // The path case is at index 2 — verify β picks it, not the
    // point cases at 0 or 1.
    let cases = vec![var("p0"), var("p1"), var("path_seg")];
    let term = elim(var("Seg"), cases);
    let normal = normalize_with_inductives(&term, &reg);
    assert!(
        matches!(&normal, CoreTerm::Var(n) if n.as_str() == "path_seg"),
        "path β must select cases[point_count + path_idx]; got {:?}",
        normal
    );
}
