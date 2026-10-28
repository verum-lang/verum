//! Public heap-verification entry points must agree on actual command effects.
use verum_verification::separation_logic::{
    Address, HeapCommand, SepProp, Value, verify_triple, verify_triple_z3,
};
use verum_verification::vcgen::{Formula, SmtExpr, Variable};

fn cell(address: i64, value: i64) -> SepProp {
    SepProp::points_to(Address::concrete(address), Value::int(value))
}

fn check(pre: &SepProp, command: HeapCommand, post: &SepProp, valid: bool) {
    assert_eq!(verify_triple(pre, &command, post).unwrap(), valid);
    assert_eq!(verify_triple_z3(pre, &command, post).unwrap().valid, valid);
}

fn equals(name: &str, value: i64) -> SepProp {
    SepProp::pure(Formula::Eq(
        Box::new(SmtExpr::var(name)),
        Box::new(SmtExpr::IntConst(value)),
    ))
}

#[test]
fn stores_preserve_exact_values_and_the_separate_frame() {
    let pre = SepProp::sep_conj(cell(8, 1), cell(16, 2));
    let command = HeapCommand::Store(Address::concrete(8), Value::int(3));
    check(
        &pre,
        command.clone(),
        &SepProp::sep_conj(cell(8, 3), cell(16, 2)),
        true,
    );
    check(
        &pre,
        command.clone(),
        &SepProp::sep_conj(cell(8, 1), cell(16, 2)),
        false,
    );
    check(
        &pre,
        command.clone(),
        &SepProp::sep_conj(cell(8, 3), cell(16, 9)),
        false,
    );
    check(&pre, command, &cell(8, 3), false);
}

#[test]
fn unowned_access_is_a_failed_obligation_even_with_a_true_postcondition() {
    let truth = SepProp::pure(Formula::True);
    for command in [
        HeapCommand::Load(Variable::new("x"), Address::concrete(8)),
        HeapCommand::Store(Address::concrete(8), Value::int(1)),
        HeapCommand::Free(Address::concrete(8)),
    ] {
        check(&SepProp::Emp, command, &truth, false);
    }
}

#[test]
fn loads_assign_the_selected_value_and_evaluate_address_before_assignment() {
    check(
        &cell(8, 42),
        HeapCommand::Load(Variable::new("x"), Address::concrete(8)),
        &equals("x", 42),
        true,
    );
    check(
        &cell(8, 42),
        HeapCommand::Load(Variable::new("x"), Address::concrete(8)),
        &equals("x", 9),
        false,
    );
    let pre = SepProp::points_to(Address::symbolic("x"), Value::int(3));
    check(
        &pre,
        HeapCommand::Load(Variable::new("x"), Address::symbolic("x")),
        &equals("x", 3),
        true,
    );
}

#[test]
fn freeing_removes_only_the_owned_cell() {
    check(
        &cell(8, 1),
        HeapCommand::Free(Address::concrete(8)),
        &SepProp::Emp,
        true,
    );
    check(
        &cell(8, 1),
        HeapCommand::Free(Address::concrete(8)),
        &cell(8, 1),
        false,
    );
    check(
        &SepProp::sep_conj(cell(8, 1), cell(16, 2)),
        HeapCommand::Free(Address::concrete(8)),
        &cell(16, 2),
        true,
    );
}

#[test]
fn allocation_preserves_old_values_and_requires_a_fresh_address() {
    let allocated = SepProp::points_to(Address::symbolic("x"), Value::int(42));
    check(
        &SepProp::Emp,
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        &allocated,
        true,
    );
    check(
        &cell(8, 1),
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        &SepProp::sep_conj(cell(8, 1), allocated),
        true,
    );
    check(
        &cell(8, 1),
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        &equals("x", 8),
        false,
    );
    check(
        &SepProp::Emp,
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        &equals("x", 8),
        false,
    );
    // Pure true permits a completely occupied domain. Lack of a fresh
    // address must fail safety, not erase all execution paths vacuously.
    let truth = SepProp::pure(Formula::True);
    check(
        &truth,
        HeapCommand::Alloc(Variable::new("x"), Value::int(42)),
        &truth,
        false,
    );
}

#[test]
fn bound_variables_shadow_load_destinations_without_capturing_them() {
    let inner = SepProp::Exists(
        [Variable::new("x")].into_iter().collect(),
        Box::new(equals("x", 9)),
    );
    check(
        &cell(8, 42),
        HeapCommand::Load(Variable::new("x"), Address::concrete(8)),
        &inner,
        true,
    );
    let duplicate = SepProp::Exists(
        [Variable::new("x"), Variable::new("x")]
            .into_iter()
            .collect(),
        Box::new(equals("x", 9)),
    );
    assert!(
        verify_triple(
            &cell(8, 42),
            &HeapCommand::Load(Variable::new("x"), Address::concrete(8)),
            &duplicate
        )
        .is_err()
    );
}

#[test]
fn separating_ownership_is_exact_and_existential_witnesses_are_local() {
    let variable = Variable::new("v");
    let pre = SepProp::Exists(
        [variable.clone()].into_iter().collect(),
        Box::new(SepProp::points_to(
            Address::concrete(8),
            Value::Symbolic(variable),
        )),
    );
    check(
        &pre,
        HeapCommand::Store(Address::concrete(8), Value::int(3)),
        &cell(8, 3),
        true,
    );
    check(
        &cell(8, 3),
        HeapCommand::Store(Address::concrete(8), Value::int(3)),
        &pre,
        true,
    );
    check(
        &cell(8, 3),
        HeapCommand::Store(Address::concrete(8), Value::int(3)),
        &SepProp::sep_conj(cell(8, 3), cell(8, 3)),
        false,
    );
}

#[test]
fn magic_wand_quantifies_disjoint_extensions() {
    check(
        &cell(8, 1),
        HeapCommand::Store(Address::concrete(8), Value::int(1)),
        &SepProp::magic_wand(SepProp::Emp, cell(8, 1)),
        true,
    );
    check(
        &cell(8, 1),
        HeapCommand::Store(Address::concrete(8), Value::int(1)),
        &SepProp::magic_wand(cell(16, 2), SepProp::Emp),
        false,
    );
}

#[test]
fn absent_layouts_and_value_representations_cannot_produce_proofs() {
    let command = HeapCommand::Store(Address::concrete(8), Value::int(1));
    for unsupported in [
        SepProp::field_points_to(Address::concrete(8), "field", Value::int(1)),
        SepProp::points_to(
            Address::concrete(8),
            Value::Struct(
                "Record".into(),
                [Value::int(1), Value::int(2)].into_iter().collect(),
            ),
        ),
        SepProp::points_to(Address::concrete(8), Value::bool(true)),
    ] {
        assert!(verify_triple(&unsupported, &command, &cell(8, 1)).is_err());
        assert!(verify_triple_z3(&cell(8, 1), &command, &unsupported).is_err());
    }
}
