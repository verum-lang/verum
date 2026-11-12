use super::{Bindings, MAX_LOAN_SHAPE_STEPS, Node, Type, TypeChecker};
use verum_common::List;

#[test]
fn wide_storage_cannot_bypass_the_work_budget() {
    let checker = TypeChecker::with_minimal_context();
    let fields: List<_> = std::iter::repeat_n(Type::Int, MAX_LOAN_SHAPE_STEPS - 1).collect();
    assert!(!checker.carries_stored_loan(&Type::Tuple(fields.clone())));
    let mut oversized = fields;
    oversized.push(Type::Int);
    assert!(checker.carries_stored_loan(&Type::Tuple(oversized)));
    assert!(checker.carries_stored_loan(&Type::Tuple(List::from_iter([
        Type::Int,
        Type::reference(false, Type::Int),
    ]))));
}

#[test]
fn cyclic_bindings_cannot_prove_reference_free_storage() {
    let checker = TypeChecker::with_minimal_context();
    let slot = Type::Var(crate::ty::TypeVar::fresh());
    let scopes: List<Bindings<'_>> = List::from_iter([
        List::from_iter([(
            &slot,
            Node {
                ty: &slot,
                scope: 1,
            },
        )]),
        List::from_iter([(
            &slot,
            Node {
                ty: &slot,
                scope: 0,
            },
        )]),
    ]);
    let mut remaining = MAX_LOAN_SHAPE_STEPS;
    assert!(
        checker
            .loan_node(
                Node {
                    ty: &slot,
                    scope: 0
                },
                &scopes,
                &mut remaining
            )
            .is_none()
    );
    assert_eq!(remaining, 0);
}
