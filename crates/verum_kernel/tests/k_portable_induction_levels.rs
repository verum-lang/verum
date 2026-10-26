use verum_kernel::proof_checker::{Certificate, DataField, Level, Term};
use verum_kernel::proof_checker_meta::{shift_universes, verify_certificate_with_lift};

#[test]
fn universe_translation_visits_inductive_declarations_and_nested_value_fields() {
    let inner = Term::Data {
        name: "Inner".into(),
        level: Level::Concrete(1),
        constructors: vec![vec![DataField::Value(Box::new(Term::universe(0)))]],
    };
    let outer = Term::Data {
        name: "Outer".into(),
        level: Level::Concrete(2),
        constructors: vec![
            vec![DataField::Value(Box::new(inner))],
            vec![DataField::Recursive],
        ],
    };
    let lifted = shift_universes(&outer, 3);
    let Term::Data {
        level,
        constructors,
        ..
    } = &lifted
    else {
        panic!("declaration lost")
    };
    assert_eq!(*level, Level::Concrete(5));
    let DataField::Value(inner) = &constructors[0][0] else {
        panic!("field lost")
    };
    let Term::Data {
        level,
        constructors,
        ..
    } = inner.as_ref()
    else {
        panic!("nested declaration lost")
    };
    assert_eq!(*level, Level::Concrete(4));
    assert_eq!(
        constructors[0][0],
        DataField::Value(Box::new(Term::universe(3)))
    );
    let cert = Certificate {
        term: outer,
        claimed_type: Term::universe(2),
        metadata: Default::default(),
    };
    assert!(verify_certificate_with_lift(&cert, 3).is_ok());
}

#[test]
fn universe_translation_preserves_constructor_ownership_and_induction() {
    let nat = Term::Data {
        name: "Nat".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![], vec![DataField::Recursive]],
    };
    let zero = Term::Construct {
        data: Box::new(nat.clone()),
        constructor: 0,
        args: vec![],
    };
    let copy = Term::lam(
        nat.clone(),
        Term::Induct {
            scrutinee: Box::new(Term::var(0)),
            motive: Box::new(Term::lam(nat.clone(), nat.clone())),
            cases: vec![
                zero,
                Term::lam(
                    nat.clone(),
                    Term::lam(
                        nat.clone(),
                        Term::Construct {
                            data: Box::new(nat.clone()),
                            constructor: 1,
                            args: vec![Term::var(0)],
                        },
                    ),
                ),
            ],
        },
    );
    let cert = Certificate {
        term: copy,
        claimed_type: Term::pi(nat.clone(), nat),
        metadata: Default::default(),
    };
    for level in [0, 1, 4] {
        assert!(verify_certificate_with_lift(&cert, level).is_ok());
    }
}
