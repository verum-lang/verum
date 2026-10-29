use verum_proof::{
    admission,
    proof_checker::{Certificate, Level, Term},
    proof_checker_nbe as nbe,
};

fn bind(domains: &[Term], body: Term, binder: fn(Term, Term) -> Term) -> Term {
    domains
        .iter()
        .rev()
        .fold(body, |body, domain| binder(domain.clone(), body))
}
fn certificate(domains: &[Term], term: Term, claim: Term) -> Certificate {
    Certificate {
        term: bind(domains, term, Term::lam),
        claimed_type: bind(domains, claim, Term::pi),
        metadata: Default::default(),
    }
}
fn apply2(f: Term, x: Term, y: Term) -> Term {
    Term::app(Term::app(f, x), y)
}

// A, a:A, P:(y:A)->Id(A,a,y)->Type(v), d:P(a,refl(a)), b:A, p:Id(A,a,b).
fn domains(u: Level, v: Level) -> [Term; 6] {
    [
        Term::Universe(u),
        Term::var(0),
        Term::pi(
            Term::var(1),
            Term::pi(
                Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
                Term::Universe(v),
            ),
        ),
        apply2(Term::var(0), Term::var(1), Term::refl(Term::var(1))),
        Term::var(3),
        Term::id_ty(Term::var(4), Term::var(3), Term::var(0)),
    ]
}
fn general(u: Level, v: Level) -> Certificate {
    certificate(
        &domains(u, v),
        Term::id_elim(Term::var(3), Term::var(2), Term::var(0)),
        apply2(Term::var(3), Term::var(1), Term::var(0)),
    )
}
fn body(term: &mut Term) -> &mut Term {
    match term {
        Term::Lam(_, next) => body(next),
        _ => term,
    }
}

// Transporting refl(a) in the family y ↦ Id(A,a,y) recovers the
// supplied path p, propositionally. The motive must mention p itself.
fn transport_recovers_path() -> Certificate {
    let ds = [
        Term::universe(0),
        Term::var(0),
        Term::var(1),
        Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
    ];
    let family = |carrier: usize, origin: usize| {
        Term::lam(
            Term::var(carrier),
            Term::id_ty(Term::var(carrier + 1), Term::var(origin + 1), Term::var(0)),
        )
    };
    let motive = Term::lam(
        Term::var(3),
        Term::lam(
            Term::id_ty(Term::var(4), Term::var(3), Term::var(0)),
            Term::id_ty(
                Term::id_ty(Term::var(5), Term::var(4), Term::var(1)),
                Term::j(family(5, 4), Term::refl(Term::var(4)), Term::var(0)),
                Term::var(0),
            ),
        ),
    );
    certificate(
        &ds,
        Term::id_elim(motive, Term::refl(Term::refl(Term::var(2))), Term::var(0)),
        Term::id_ty(
            Term::id_ty(Term::var(3), Term::var(2), Term::var(1)),
            Term::j(family(3, 2), Term::refl(Term::var(2)), Term::var(0)),
            Term::var(0),
        ),
    )
}
fn verdict(cert: &Certificate, expected: bool) {
    assert_eq!(
        cert.verify().is_ok(),
        expected,
        "substitution: {:?}",
        cert.verify()
    );
    assert_eq!(
        nbe::verify_certificate(cert).is_ok(),
        expected,
        "NbE: {:?}",
        nbe::verify_certificate(cert)
    );
    let judgment = admission::Judgment {
        profile: admission::PROFILE.into(),
        statement: cert.claimed_type.clone(),
        certificate: cert.clone(),
    };
    assert_eq!(
        admission::replay(&serde_json::to_vec(&judgment).unwrap()).accepted,
        expected
    );
}

fn battery() -> Vec<(&'static str, Certificate, bool)> {
    let good = general(Level::Concrete(0), Level::Concrete(1));
    let ds = domains(Level::Concrete(0), Level::Concrete(1));
    let refl_a = Term::refl(Term::var(4)); // a in the full six-binder frame
    let beta = certificate(
        &ds,
        Term::refl(Term::var(2)),
        Term::id_ty(
            apply2(Term::var(3), Term::var(4), refl_a.clone()),
            Term::id_elim(Term::var(3), Term::var(2), refl_a),
            Term::var(2),
        ),
    );
    let mut cases = vec![
        ("general", good.clone(), true),
        ("beta", beta, true),
        (
            "symbolic_universes",
            general(Level::var("u"), Level::var("v")),
            true,
        ),
    ];
    cases.push(("transport_recovers_path", transport_recovers_path(), true));
    let mut distinct_paths = ds.to_vec();
    distinct_paths.push(Term::id_ty(Term::var(5), Term::var(4), Term::var(1)));
    cases.push((
        "result_retains_the_supplied_proof",
        certificate(
            &distinct_paths,
            Term::id_elim(Term::var(4), Term::var(3), Term::var(1)),
            apply2(Term::var(4), Term::var(2), Term::var(0)),
        ),
        false,
    ));
    for (name, replacement) in [
        (
            "wrong_base",
            Term::id_elim(Term::var(3), Term::var(1), Term::var(0)),
        ),
        (
            "wrong_path_origin",
            Term::id_elim(Term::var(3), Term::var(2), Term::refl(Term::var(1))),
        ),
        (
            "non_path",
            Term::id_elim(Term::var(3), Term::var(2), Term::var(1)),
        ),
        (
            "missing_proof_binder",
            Term::id_elim(
                Term::lam(Term::var(5), Term::universe(0)),
                Term::var(2),
                Term::var(0),
            ),
        ),
        (
            "legacy_transport_is_not_full_induction",
            Term::j(Term::var(3), Term::var(2), Term::var(0)),
        ),
    ] {
        let mut cert = good.clone();
        *body(&mut cert.term) = replacement;
        cases.push((name, cert, false));
    }
    // A well-formed motive with a reversed, fixed, or wrong-carrier path
    // binder must not become based induction merely because its base exists.
    let env = [
        Term::universe(0),
        Term::var(0),
        Term::var(1),
        Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
    ];
    let expected_domain = Term::id_ty(Term::var(4), Term::var(3), Term::var(0));
    for (name, path_domain) in [
        (
            "reversed_proof_domain",
            Term::id_ty(Term::var(4), Term::var(0), Term::var(3)),
        ),
        (
            "fixed_loop_domain",
            Term::id_ty(Term::var(4), Term::var(3), Term::var(3)),
        ),
        ("unrelated_proof_domain", Term::var(4)),
    ] {
        let motive = Term::lam(Term::var(3), Term::lam(path_domain, Term::var(5)));
        cases.push((
            name,
            certificate(
                &env,
                Term::id_elim(motive, Term::var(2), Term::var(0)),
                Term::var(3),
            ),
            false,
        ));
    }
    // A family returning an inhabited value rather than a type is not a motive.
    let bad_kind = Term::lam(Term::var(3), Term::lam(expected_domain, Term::var(4)));
    cases.push((
        "motive_returns_value",
        certificate(
            &env,
            Term::id_elim(bad_kind, Term::var(2), Term::var(0)),
            Term::var(3),
        ),
        false,
    ));

    // A, a, p:Id(A,a,a). Even a loop remains neutral. No K/UIP/eta for paths.
    let loops = [
        Term::universe(0),
        Term::var(0),
        Term::id_ty(Term::var(1), Term::var(0), Term::var(0)),
    ];
    let motive = Term::lam(
        Term::var(2),
        Term::lam(
            Term::id_ty(Term::var(3), Term::var(2), Term::var(0)),
            Term::var(4),
        ),
    );
    let elimination = Term::id_elim(motive, Term::var(1), Term::var(0));
    cases.push((
        "neutral_loop_does_not_compute",
        certificate(
            &loops,
            Term::refl(Term::var(1)),
            Term::id_ty(Term::var(2), elimination, Term::var(1)),
        ),
        false,
    ));
    cases.push((
        "no_uniqueness_of_identity_proofs",
        certificate(
            &loops,
            Term::refl(Term::var(0)),
            Term::id_ty(
                Term::id_ty(Term::var(2), Term::var(1), Term::var(1)),
                Term::var(0),
                Term::refl(Term::var(1)),
            ),
        ),
        false,
    ));
    cases
}

#[test]
fn dependent_identity_induction_and_adversarial_rules_agree() {
    for (name, cert, expected) in battery() {
        eprintln!("{name}");
        verdict(&cert, expected);
    }
}

#[test]
fn conversion_substitution_and_closure_capture_preserve_proof_arguments() {
    let cert = general(Level::Concrete(2), Level::Concrete(4));
    verdict(&cert, true);
    // Beta-substitution through every eliminator child, and a transparent
    // let with unused bindings exercise NbE environment compaction/readback.
    let mut wrapped = cert.clone();
    *body(&mut wrapped.term) = Term::app(
        Term::lam(
            Term::var(5),
            Term::id_elim(Term::var(4), Term::var(3), Term::var(1)),
        ),
        Term::var(4),
    );
    verdict(&wrapped, true);
    *body(&mut wrapped.term) = Term::Let {
        value: Box::new(Term::var(3)),
        body: Box::new(Term::id_elim(Term::var(0), Term::var(3), Term::var(1))),
    };
    verdict(&wrapped, true);
    let normal = nbe::quote(&nbe::eval(&wrapped.term, &vec![]), 0);
    verdict(
        &Certificate {
            term: normal,
            ..cert
        },
        true,
    );
}

#[test]
fn structural_admission_checks_each_eliminator_child_without_implicit_binders() {
    let cert = general(Level::Concrete(0), Level::Concrete(0));
    for child in ["motive", "base", "scrutinee"] {
        let mut malformed = cert.clone();
        let Term::IdElim {
            motive,
            base,
            scrutinee,
        } = body(&mut malformed.term)
        else {
            panic!("eliminator")
        };
        let selected = match child {
            "motive" => motive,
            "base" => base,
            _ => scrutinee,
        };
        **selected = Term::var(6); // only six outer binders; no implicit locals
        verdict(&malformed, false);
    }
    let wire = serde_json::to_value(Term::id_elim(
        Term::universe(0),
        Term::universe(0),
        Term::universe(0),
    ))
    .unwrap();
    let mut extra = wire.clone();
    extra["IdElim"]["trusted"] = true.into();
    assert!(serde_json::from_value::<Term>(extra).is_err());
    let mut missing = wire;
    missing["IdElim"]
        .as_object_mut()
        .unwrap()
        .remove("scrutinee");
    assert!(serde_json::from_value::<Term>(missing).is_err());
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_replay_of_dependent_identity_induction() {
    let executable = std::env::var_os("VERUM_LEAN_REPLAY").expect("build independent checker");
    let cases = battery();
    let rows: Vec<_> = cases
        .iter()
        .map(|(id, cert, expected)| {
            verdict(cert, *expected);
            serde_json::json!({"id":id,"term":cert.term,"claimed_type":cert.claimed_type})
        })
        .collect();
    let path = std::env::temp_dir().join(format!(
        "verum-identity-induction-{}.json",
        std::process::id()
    ));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(&executable)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = report["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (actual, (id, _, expected)) in verdicts.iter().zip(cases) {
        assert_eq!(actual["id"], id);
        assert_eq!(actual["ok"], expected, "{actual}");
    }
    // Reject invented authority fields and missing children before checking.
    for extra in [true, false] {
        let mut malformed = serde_json::to_value(Term::id_elim(
            Term::universe(0),
            Term::universe(0),
            Term::universe(0),
        ))
        .unwrap();
        if extra {
            malformed["IdElim"]["trusted"] = true.into();
        } else {
            malformed["IdElim"].as_object_mut().unwrap().remove("base");
        }
        let wire = serde_json::json!({"schema_version":1,"certificates":[{
            "id":"malformed", "term":malformed, "claimed_type":Term::universe(1)
        }]});
        std::fs::write(&path, serde_json::to_vec(&wire).unwrap()).unwrap();
        let rejected = std::process::Command::new(&executable)
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(rejected.status.code(), Some(4));
    }
    std::fs::remove_file(path).unwrap();
}
