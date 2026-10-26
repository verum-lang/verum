use verum_kernel::proof_checker::{DataField, Level, Term};
use verum_kernel::{ReflectedJudgment, ReflectedTerm, is_reflected_well_formed};

fn nat() -> Term {
    Term::Data {
        name: "Nat".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![], vec![DataField::Recursive]],
    }
}
fn zero() -> Term {
    Term::Construct {
        data: Box::new(nat()),
        constructor: 0,
        args: vec![],
    }
}

#[test]
fn reflection_round_trips_declarations_owned_constructors_induction_and_definitions() {
    let generic = Term::lam(
        Term::universe(0),
        Term::Data {
            name: "List".into(),
            level: Level::Concrete(0),
            constructors: vec![
                vec![],
                vec![
                    DataField::Value(Box::new(Term::var(0))),
                    DataField::Recursive,
                ],
            ],
        },
    );
    let induction = Term::Induct {
        scrutinee: Box::new(zero()),
        motive: Box::new(Term::lam(nat(), nat())),
        cases: vec![zero(), Term::lam(nat(), Term::lam(nat(), Term::var(0)))],
    };
    let definition = Term::Let {
        value: Box::new(induction.clone()),
        body: Box::new(Term::var(0)),
    };
    for original in [generic, zero(), induction, definition] {
        let reflected = ReflectedTerm::from(&original);
        assert_eq!(Term::try_from(&reflected).unwrap(), original);
        let json = serde_json::to_string(&reflected).unwrap();
        let decoded: ReflectedTerm = serde_json::from_str(&json).unwrap();
        assert_eq!(Term::try_from(&decoded).unwrap(), original);
        assert!(is_reflected_well_formed(&ReflectedJudgment::closed(
            reflected,
            ReflectedTerm::from(&Term::universe(1))
        )));
    }
}

#[test]
fn reflection_checks_captured_field_variables_and_local_definition_scope() {
    let open = Term::Data {
        name: "Open".into(),
        level: Level::Concrete(0),
        constructors: vec![vec![DataField::Value(Box::new(Term::var(0)))]],
    };
    for bad in [
        open,
        Term::Let {
            value: Box::new(Term::var(0)),
            body: Box::new(Term::var(0)),
        },
    ] {
        assert!(!is_reflected_well_formed(&ReflectedJudgment::closed(
            ReflectedTerm::from(&bad),
            ReflectedTerm::from(&Term::universe(1))
        )));
    }
}
