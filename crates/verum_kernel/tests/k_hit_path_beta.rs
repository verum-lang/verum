//! Path names are local variables, not evidence of a checked higher cell.
//! Until path introduction and coherence are represented and checked, a bare
//! variable must remain neutral even when it shares a declaration's path name.

use verum_common::{Heap, List, Text};
use verum_kernel::{
    ConstructorSig, CoreTerm, InductiveRegistry, PathCtorSig, RegisteredInductive,
    normalize_with_inductives,
};

fn nullary(name: &str) -> ConstructorSig {
    ConstructorSig {
        name: Text::from(name),
        arg_types: List::new(),
    }
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
fn s1_loop_spelling_does_not_reduce_a_local_variable() {
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
    assert_eq!(
        normal, term,
        "path spelling cannot establish constructor identity"
    );
}

#[test]
fn interval_segment_spelling_does_not_reduce_a_local_variable() {
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
    assert_eq!(
        normal, term,
        "path spelling cannot establish constructor identity"
    );
}

#[test]
fn suspension_meridian_spelling_does_not_reduce_a_local_variable() {
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
    assert_eq!(
        normal, term,
        "path spelling cannot establish constructor identity"
    );
}

#[test]
fn multiple_path_names_remain_local_variables() {
    // Multiple declared paths do not turn local variables into path constructors.
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
    assert_eq!(
        normal_a, term_a,
        "path spelling cannot establish constructor identity"
    );

    let term_b = elim(var("loop_b"), cases.clone());
    let normal_b = normalize_with_inductives(&term_b, &reg);
    assert_eq!(
        normal_b, term_b,
        "path spelling cannot establish constructor identity"
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
fn path_name_does_not_select_a_branch_by_roster_offset() {
    // HIT with TWO point ctors and ONE path ctor:
    //  cases = [point0, point1, path0]
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

    // The old spelling-based reduction incorrectly selected index 2 here.
    let cases = vec![var("p0"), var("p1"), var("path_seg")];
    let term = elim(var("Seg"), cases);
    let normal = normalize_with_inductives(&term, &reg);
    assert_eq!(
        normal, term,
        "path spelling cannot establish constructor identity"
    );
}
