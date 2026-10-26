use verum_proof::{
    admission::*,
    proof_checker::{Certificate, Term},
};
fn identity() -> Judgment {
    let statement = Term::pi(Term::universe(0), Term::pi(Term::var(0), Term::var(1)));
    Judgment {
        profile: PROFILE.into(),
        statement: statement.clone(),
        certificate: Certificate {
            term: Term::lam(Term::universe(0), Term::lam(Term::var(0), Term::var(0))),
            claimed_type: statement,
            metadata: Default::default(),
        },
    }
}
fn check(j: &Judgment) -> Report {
    replay(&serde_json::to_vec(j).unwrap())
}
#[test]
fn exact_polymorphic_judgment_replays_and_does_not_trust_names() {
    let mut j = identity();
    assert!(check(&j).accepted);
    j.certificate.metadata.insert(
        "theorem".into(),
        "arbitrary false prose is not a formal claim".into(),
    );
    assert!(check(&j).accepted);
    j.statement = Term::universe(0);
    assert_eq!(check(&j).diagnostic.unwrap().code, "statement_mismatch");
}
#[test]
fn unknown_and_duplicate_fields_cannot_smuggle_an_authority() {
    let data = serde_json::to_string(&identity()).unwrap();
    for source in [
        format!("{{\"certified\":true,{}", &data[1..]),
        format!("{{\"profile\":\"fake\",{}", &data[1..]),
    ] {
        assert_eq!(
            replay(source.as_bytes()).diagnostic.unwrap().code,
            "invalid_certificate"
        );
    }
    let mut json = serde_json::to_value(identity()).unwrap();
    json["certificate"]["axioms"] = serde_json::json!(["false"]);
    assert!(!replay(&serde_json::to_vec(&json).unwrap()).accepted);
}
#[test]
fn ill_formed_goal_and_free_variables_fail_with_diagnostics() {
    let mut j = identity();
    j.certificate.term = Term::var(0);
    assert_eq!(check(&j).diagnostic.unwrap().code, "unbound_variable");
    j.certificate.claimed_type = Term::app(
        Term::lam(Term::universe(0), Term::var(0)),
        Term::universe(1),
    );
    j.statement = j.certificate.claimed_type.clone();
    j.certificate.term = Term::universe(0);
    assert!(!check(&j).accepted);
}
#[test]
fn profile_and_resource_refusals_are_never_acceptance() {
    let mut j = identity();
    j.profile = "unregistered-logic".into();
    assert_eq!(check(&j).diagnostic.unwrap().code, "unsupported_profile");
    assert_eq!(
        replay(&vec![b' '; MAX_BYTES + 1]).diagnostic.unwrap().code,
        "byte_limit"
    );
    let id = checker_identity();
    assert_eq!(id.len(), 64);
    assert_eq!(id, checker_identity());
}

#[test]
fn wire_wrappers_do_not_reduce_the_logical_term_depth_budget() {
    use verum_proof::proof_checker::{DataField, Level};
    let make = |depth| {
        let mut data = Term::Data {
            name: "Leaf".into(),
            level: Level::Concrete(0),
            constructors: vec![vec![]],
        };
        for _ in 0..depth {
            data = Term::Data {
                name: "Nested".into(),
                level: Level::Concrete(0),
                constructors: vec![vec![DataField::Value(Box::new(data))]],
            };
        }
        Judgment {
            profile: PROFILE.into(),
            statement: Term::universe(0),
            certificate: Certificate {
                term: data,
                claimed_type: Term::universe(0),
                metadata: Default::default(),
            },
        }
    };
    let bytes = serde_json::to_vec(&make(30)).unwrap();
    assert!(
        serde_json::from_slice::<Judgment>(&bytes).is_err(),
        "fixture must exceed serde's wrapper-based default"
    );
    assert!(
        replay(&bytes).accepted,
        "logical depth is safely inside the advertised limit"
    );
    let too_deep = replay(&serde_json::to_vec(&make(65)).unwrap());
    assert!(!too_deep.accepted);
    assert_eq!(too_deep.diagnostic.unwrap().code, "structural_limit");
}

#[test]
fn json_preflight_bounds_nesting_without_interpreting_quoted_delimiters() {
    let nested = format!("{}0{}", "[".repeat(400), "]".repeat(400));
    assert_eq!(
        replay(nested.as_bytes()).diagnostic.unwrap().code,
        "json_depth_limit"
    );
    let mut j = Judgment {
        profile: PROFILE.into(),
        statement: Term::universe(1),
        certificate: Certificate {
            term: Term::universe(0),
            claimed_type: Term::universe(1),
            metadata: Default::default(),
        },
    };
    j.certificate
        .metadata
        .insert("delimiters".into(), "{[\\\"".repeat(500));
    assert!(replay(&serde_json::to_vec(&j).unwrap()).accepted);
    let mut trailing = serde_json::to_vec(&j).unwrap();
    trailing.extend_from_slice(b" {}");
    assert_eq!(
        replay(&trailing).diagnostic.unwrap().code,
        "invalid_certificate"
    );
}
