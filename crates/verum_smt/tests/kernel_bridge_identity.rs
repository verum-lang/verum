use verum_common::Text;
use verum_kernel::proof_checker::{DataField, Level, Term};
use verum_kernel::separation_logic::HeapPredicate;
use verum_smt::separation_kernel_bridge::KernelSepBridge;

fn handle(term: Term) -> Text {
    Text::from(format!(
        "{:?}",
        KernelSepBridge::lift(&HeapPredicate::Pure(term))
    ))
}

#[test]
fn opaque_handles_preserve_payloads_and_declaration_identity() {
    let nat = Term::Data {
        name: "Nat".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![], vec![DataField::Recursive]]
            .into_iter()
            .map(Into::into)
            .collect(),
    };
    let zero = Term::Construct {
        data: Box::new(nat.clone()),
        constructor: 0,
        args: vec![],
    };
    let successor = Term::Construct {
        data: Box::new(nat.clone()),
        constructor: 1,
        args: vec![zero.clone()],
    };
    assert_ne!(handle(zero.clone()), handle(successor));
    let same_name_different_declaration = Term::Data {
        name: "Nat".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![]].into_iter().map(Into::into).collect(),
    };
    assert_ne!(handle(nat.clone()), handle(same_name_different_declaration));
    assert_eq!(handle(nat.clone()), handle(nat));
    assert_ne!(
        handle(Term::app(Term::var(0), Term::var(1))),
        handle(Term::app(Term::var(0), Term::var(2)))
    );
    assert_ne!(
        handle(Term::Let {
            value: Box::new(zero),
            body: Box::new(Term::var(0))
        }),
        handle(Term::var(0))
    );
}
