#[path = "support/inductives.rs"]
mod fixtures;
use verum_proof::proof_checker::{Certificate, Term};

#[test]
fn native_induction_checks_complete_declarations_cases_and_exact_statements() {
    for (name, cert, expected) in fixtures::battery() {
        let substitution = cert.verify();
        let nbe = verum_proof::proof_checker_nbe::verify_certificate(&cert);
        assert_eq!(
            substitution.is_ok(),
            expected,
            "substitution: {name}: {substitution:?}"
        );
        assert_eq!(nbe.is_ok(), expected, "NbE: {name}: {nbe:?}");
        let encoded = serde_json::to_vec(&cert).unwrap();
        let decoded: Certificate = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.term, cert.term);
    }
}

#[test]
fn captured_type_parameters_survive_application() {
    let list = fixtures::list(Term::var(1));
    let body = fixtures::ctor(
        list.clone(),
        1,
        vec![Term::var(0), fixtures::ctor(list.clone(), 0, vec![])],
    );
    let term = Term::lam(Term::universe(0), Term::lam(Term::var(0), body));
    let claim = Term::pi(Term::universe(0), Term::pi(Term::var(0), list));
    let cert = fixtures::certificate(term.clone(), claim);
    assert!(cert.verify().is_ok());
    assert!(verum_proof::proof_checker_nbe::verify_certificate(&cert).is_ok());
    let value = Term::app(Term::app(term, fixtures::nat()), fixtures::zero());
    assert!(
        fixtures::certificate(value, fixtures::list(fixtures::nat()))
            .verify()
            .is_ok()
    );
}

#[test]
fn inductive_wire_schema_rejects_invented_recursion_forms_and_extra_fields() {
    let mut raw = serde_json::to_value(fixtures::nat()).unwrap();
    raw["Data"]["constructors"][1][0] = serde_json::json!({"Negative":"Recursive"});
    assert!(serde_json::from_value::<Term>(raw).is_err());
    let mut raw = serde_json::to_value(fixtures::nat()).unwrap();
    raw["Data"]["assume_positive"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Term>(raw).is_err());
}

#[test]
fn admission_counts_constructor_and_recursive_slot_nodes() {
    use verum_proof::admission::{Judgment, MAX_NODES, PROFILE, replay};
    use verum_proof::proof_checker::Level;
    let declaration = Term::Data {
        name: "ManyConstructors".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![]; MAX_NODES + 1]
            .into_iter()
            .map(Into::into)
            .collect(),
    };
    let cert = fixtures::certificate(declaration, Term::universe(0));
    let judgment = Judgment {
        profile: PROFILE.into(),
        statement: cert.claimed_type.clone(),
        certificate: cert,
    };
    let result = replay(&serde_json::to_vec(&judgment).unwrap());
    assert!(!result.accepted);
    assert_eq!(result.diagnostic.unwrap().code, "structural_limit");
}

#[test]
fn admission_scopes_dependent_fields_without_admitting_self_references() {
    use verum_proof::admission::{Judgment, PROFILE, replay};
    use verum_proof::proof_checker::{DataField, Level};
    for (index, expected) in [(0, true), (1, false)] {
        let declaration = Term::Data {
            name: "Package".into(),
            level: Level::Concrete(1),
            indices: vec![],
            constructors: vec![vec![
                DataField::Value(Box::new(Term::universe(0))),
                DataField::Dependent(Box::new(Term::var(index))),
            ]]
            .into_iter()
            .map(Into::into)
            .collect(),
        };
        let certificate = fixtures::certificate(declaration, Term::universe(1));
        let report = replay(
            &serde_json::to_vec(&Judgment {
                profile: PROFILE.into(),
                statement: certificate.claimed_type.clone(),
                certificate,
            })
            .unwrap(),
        );
        assert_eq!(report.accepted, expected, "{report:?}");
        if !expected {
            assert_eq!(report.diagnostic.unwrap().code, "unbound_variable");
        }
    }
}
