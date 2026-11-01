use verum_proof::{
    admission::{ReplayOutcome, classify_check_failure},
    proof_checker::{Certificate, DataField, Level, Term},
    proof_checker_nbe,
};
use verum_source_proof::{finite::proof::certificate, scoped::Error};

fn finite(name: &str, constructors: usize) -> Term {
    Term::Data {
        name: name.into(),
        level: Level::Concrete(0),
        indices: Vec::new(),
        constructors: (0..constructors).map(|_| Vec::new().into()).collect(),
    }
}
fn value(domain: &Term, constructor: usize) -> Term {
    Term::Construct {
        data: Box::new(domain.clone()),
        constructor,
        args: Vec::new(),
    }
}
fn select(domain: &Term, argument: Term, result: &Term, cases: Vec<Term>) -> Term {
    Term::Induct {
        scrutinee: Box::new(argument),
        motive: Box::new(Term::lam(domain.clone(), result.clone())),
        cases,
    }
}
fn equality(boolean: &Term, left: Term, right: Term) -> Term {
    let not_right = select(
        boolean,
        right.clone(),
        boolean,
        vec![value(boolean, 1), value(boolean, 0)],
    );
    select(boolean, left, boolean, vec![not_right, right])
}
fn verdict(certificate: &Certificate, expected: bool) {
    for result in [
        certificate.verify(),
        proof_checker_nbe::verify_certificate(certificate),
    ] {
        let outcome = result.as_ref().map_or_else(
            |error| classify_check_failure(error).0,
            |_| ReplayOutcome::Accepted,
        );
        assert_eq!(
            outcome,
            if expected {
                ReplayOutcome::Accepted
            } else {
                ReplayOutcome::Rejected
            },
            "{result:?}"
        );
    }
}

fn mixed_predicate(boolean: &Term, phase: &Term, changed: bool) -> Term {
    // Three independent inputs: first Bool, three-way phase, second Bool.
    // Every phase tests a different input-sensitive expression against itself.
    let first = Term::var(2);
    let second = Term::var(0);
    let selected = select(
        phase,
        Term::var(1),
        boolean,
        vec![
            first.clone(),
            second.clone(),
            equality(boolean, first, second.clone()),
        ],
    );
    let reference = if changed { second } else { selected.clone() };
    Term::lam(
        boolean.clone(),
        Term::lam(
            phase.clone(),
            Term::lam(boolean.clone(), equality(boolean, selected, reference)),
        ),
    )
}

#[test]
fn universal_induction_preserves_mixed_input_order_and_checks_every_constructor() {
    let boolean = finite("Bool", 2);
    let phase = finite("Phase", 3);
    let domains = [boolean.clone(), phase.clone(), boolean.clone()];
    let predicate = mixed_predicate(&boolean, &phase, false);
    let result = certificate(&domains, &predicate, &boolean).unwrap();
    let expected = Term::Let {
        value: Box::new(predicate),
        body: Box::new(Term::pi(
            boolean.clone(),
            Term::pi(
                phase.clone(),
                Term::pi(
                    boolean.clone(),
                    Term::id_ty(
                        boolean.clone(),
                        Term::app(
                            Term::app(Term::app(Term::var(3), Term::var(2)), Term::var(1)),
                            Term::var(0),
                        ),
                        value(&boolean, 1),
                    ),
                ),
            ),
        )),
    };
    assert_eq!(result.claimed_type, expected);
    verdict(&result, true);
    let altered =
        certificate(&domains, &mixed_predicate(&boolean, &phase, true), &boolean).unwrap();
    verdict(&altered, false);
}

#[test]
fn generating_a_candidate_does_not_claim_that_the_predicate_is_true() {
    let boolean = finite("Bool", 2);
    for (truth, expected) in [(0, false), (1, true)] {
        let result = certificate(&[], &value(&boolean, truth), &boolean).unwrap();
        verdict(&result, expected);
    }
    let result = certificate(
        &[boolean.clone()],
        &Term::lam(boolean.clone(), Term::var(0)),
        &boolean,
    )
    .unwrap();
    verdict(&result, false);
}

#[test]
fn incomplete_case_tree_is_rejected_by_both_native_checkers() {
    let boolean = finite("Bool", 2);
    let mut result = certificate(
        &[boolean.clone()],
        &Term::lam(boolean.clone(), value(&boolean, 1)),
        &boolean,
    )
    .unwrap();
    let Term::Let { body, .. } = &mut result.term else {
        panic!("shared predicate")
    };
    let Term::Lam(_, body) = body.as_mut() else {
        panic!("input binder")
    };
    let Term::Induct { cases, .. } = body.as_mut() else {
        panic!("finite induction")
    };
    cases.pop();
    verdict(&result, false);
}

#[test]
fn unsupported_domains_and_open_or_ill_typed_predicates_cannot_be_captured() {
    let boolean = finite("Bool", 2);
    let mut payload = finite("Payload", 1);
    if let Term::Data { constructors, .. } = &mut payload {
        constructors[0]
            .fields
            .push(DataField::Value(Box::new(boolean.clone())));
    }
    let mut indexed = finite("Indexed", 1);
    if let Term::Data { indices, .. } = &mut indexed {
        indices.push(boolean.clone());
    }
    for domain in [finite("Empty", 0), payload, indexed, Term::var(0)] {
        assert!(matches!(
            certificate(&[domain], &value(&boolean, 1), &boolean),
            Err(Error::Unsupported(_))
        ));
    }
    assert!(certificate(&[boolean.clone()], &Term::var(0), &boolean).is_err());
    assert!(certificate(&[boolean.clone()], &value(&boolean, 1), &boolean).is_err());
    assert!(certificate(&[], &value(&boolean, 1), &finite("NotBool", 3)).is_err());
}

#[test]
fn finite_leaf_and_parameter_budgets_are_inconclusive() {
    let boolean = finite("Bool", 2);
    let singleton = finite("Unit", 1);
    for domains in [vec![boolean.clone(); 13], vec![singleton; 65]] {
        assert!(matches!(
            certificate(&domains, &value(&boolean, 1), &boolean),
            Err(Error::Inconclusive {
                code: "finite_case_limit",
                ..
            })
        ));
    }
    // The exact leaf boundary is accepted by construction. The proof verifier
    // independently owns its own node, depth and evaluation resource budgets.
    let boundary = finite("Boundary", 4096);
    assert!(
        certificate(
            &[boundary.clone()],
            &Term::lam(boundary, value(&boolean, 1)),
            &boolean,
        )
        .is_ok()
    );
}
