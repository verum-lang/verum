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
fn indexed_constructor_wire_rejects_ambiguous_or_asserted_indices() {
    let declaration = serde_json::json!({"Data": {
        "name": "Unit", "level": 0, "constructors": [{"fields": [], "indices": []}]
    }});
    let mut judgment = serde_json::to_value(identity()).unwrap();
    judgment["statement"] = serde_json::json!({"Universe": 0});
    judgment["certificate"]["claimed_type"] = judgment["statement"].clone();
    judgment["certificate"]["term"] = declaration;
    assert!(replay(&serde_json::to_vec(&judgment).unwrap()).accepted);
    let valid = serde_json::to_string(&judgment).unwrap();
    for constructor in [
        r#"{"fields":[],"indices":[],"positive":true}"#,
        r#"{"fields":[],"indices":[],"indices":[]}"#,
        r#"{"fields":[],"fields":[],"indices":[]}"#,
        r#"{"fields":[]}"#,
    ] {
        let source = valid.replace(r#"{"fields":[],"indices":[]}"#, constructor);
        assert_ne!(source, valid);
        assert_eq!(
            replay(source.as_bytes()).diagnostic.unwrap().code,
            "invalid_certificate"
        );
    }
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
            indices: vec![],
            constructors: vec![vec![]].into_iter().map(Into::into).collect(),
        };
        for _ in 0..depth {
            data = Term::Data {
                name: "Nested".into(),
                level: Level::Concrete(0),
                indices: vec![],
                constructors: vec![vec![DataField::Value(Box::new(data))]]
                    .into_iter()
                    .map(Into::into)
                    .collect(),
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
    let nested = format!("{}0{}", "[".repeat(1024), "]".repeat(1024));
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

fn scope_boundary_terms() -> Vec<Term> {
    use verum_proof::proof_checker::{DataField, Level};
    let u = Term::universe(0);
    let free = Term::var(0);
    let data = Term::Data {
        name: "Unit".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![]].into_iter().map(Into::into).collect(),
    };
    let unit = Term::Construct {
        data: Box::new(data.clone()),
        constructor: 0,
        args: vec![],
    };
    vec![
        free.clone(),
        Term::lam(free.clone(), Term::var(0)),
        Term::pi(free.clone(), u.clone()),
        Term::sigma(free.clone(), u.clone()),
        Term::Let {
            value: Box::new(free.clone()),
            body: Box::new(u.clone()),
        },
        // The body has one binder; index one is still free.
        Term::lam(u.clone(), Term::var(1)),
        Term::Let {
            value: Box::new(u.clone()),
            body: Box::new(Term::var(1)),
        },
        Term::Data {
            name: "Field".into(),
            level: Level::Concrete(0),
            indices: vec![],
            constructors: vec![vec![DataField::Value(Box::new(free.clone()))]]
                .into_iter()
                .map(Into::into)
                .collect(),
        },
        Term::Construct {
            data: Box::new(data.clone()),
            constructor: 0,
            args: vec![free.clone()],
        },
        Term::Induct {
            scrutinee: Box::new(unit),
            motive: Box::new(Term::lam(data, u.clone())),
            cases: vec![free.clone()],
        },
        Term::pair(u.clone(), free.clone()),
        Term::fst(free.clone()),
        Term::snd(free.clone()),
        Term::id_ty(u.clone(), u.clone(), free.clone()),
        Term::refl(free.clone()),
        Term::j(u.clone(), free.clone(), u.clone()),
        // No reduction may erase an out-of-scope argument.
        Term::app(Term::lam(u.clone(), u.clone()), Term::var(usize::MAX)),
    ]
}

#[test]
fn scope_is_checked_before_reduction_at_every_binding_boundary() {
    let free = Term::var(0);
    for term in scope_boundary_terms() {
        let mut judgment = identity();
        judgment.certificate.term = term;
        let report = check(&judgment);
        let diagnostic = report.diagnostic.unwrap();
        assert_eq!(diagnostic.code, "unbound_variable");
        assert_eq!(diagnostic.stage, "admission");
    }
    for claimed in [false, true] {
        let mut judgment = identity();
        if claimed {
            judgment.certificate.claimed_type = free.clone();
        } else {
            judgment.statement = free.clone();
        }
        assert_eq!(
            check(&judgment).diagnostic.unwrap().code,
            "unbound_variable"
        );
    }
}

#[test]
fn captured_datatype_fields_and_nested_local_binders_remain_admissible() {
    use verum_proof::proof_checker::{DataField, Level};
    let data = Term::Data {
        name: "Capture".into(),
        level: Level::Concrete(0),
        indices: vec![],
        constructors: vec![vec![DataField::Value(Box::new(Term::var(0)))]]
            .into_iter()
            .map(Into::into)
            .collect(),
    };
    let statement = Term::pi(Term::universe(0), Term::universe(0));
    let judgment = Judgment {
        profile: PROFILE.into(),
        statement: statement.clone(),
        certificate: Certificate {
            term: Term::lam(Term::universe(0), data),
            claimed_type: statement,
            metadata: Default::default(),
        },
    };
    assert!(check(&judgment).accepted);
    let mut judgment = identity();
    judgment.certificate.term = Term::lam(
        Term::universe(0),
        Term::lam(
            Term::var(0),
            Term::Let {
                value: Box::new(Term::var(0)),
                body: Box::new(Term::var(0)),
            },
        ),
    );
    assert!(check(&judgment).accepted);
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_checker_rejects_unbound_variables_at_all_boundaries() {
    let replay =
        std::env::var_os("VERUM_LEAN_REPLAY").expect("build the independent checker first");
    let good = identity();
    let terms = scope_boundary_terms();
    let mut rows: Vec<_> = terms
        .iter()
        .enumerate()
        .map(|(id, term)| {
            serde_json::json!({
                "id": id.to_string(), "term": term, "claimed_type": good.statement,
            })
        })
        .collect();
    rows.push(
        serde_json::json!({ "id":"closed", "term":good.certificate.term,
        "claimed_type":good.statement }),
    );
    let path = std::env::temp_dir().join(format!("verum-scope-{}.json", std::process::id()));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(replay)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = report["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), terms.len() + 1);
    for (i, verdict) in verdicts[..terms.len()].iter().enumerate() {
        assert_eq!(verdict["id"], i.to_string());
        assert_eq!(verdict["ok"], false);
        assert_eq!(verdict["error"], "UnboundVariable");
    }
    assert_eq!(verdicts.last().unwrap()["id"], "closed");
    assert_eq!(verdicts.last().unwrap()["ok"], true);
}
