//! Independent executable replay, including malformed equality certificates.
use std::process::Command;
use verum_proof::proof_checker::{Certificate, Term};

fn bind(domains: &[Term], body: Term, binder: fn(Term, Term) -> Term) -> Term {
    domains
        .iter()
        .rev()
        .fold(body, |body, domain| binder(domain.clone(), body))
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the built independent checker"]
fn independent_replay_checks_equality_and_rejects_invalid_proofs() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker first");
    let u = Term::universe(0);
    let refl_domains = [u.clone(), Term::var(0)];
    let refl = bind(&refl_domains, Term::refl(Term::var(0)), Term::lam);
    let refl_type = bind(
        &refl_domains,
        Term::id_ty(Term::var(1), Term::var(0), Term::var(0)),
        Term::pi,
    );
    // A, x:A, y:A, p:Id(A,x,y), P:A→Type, h:P(x).
    let domains = [
        u.clone(),
        Term::var(0),
        Term::var(1),
        Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
        Term::pi(Term::var(3), u.clone()),
        Term::app(Term::var(0), Term::var(3)),
    ];
    let transport_type = bind(&domains, Term::app(Term::var(1), Term::var(3)), Term::pi);
    let transport = bind(
        &domains,
        Term::j(Term::var(1), Term::var(0), Term::var(2)),
        Term::lam,
    );
    let mut cases = vec![
        ("reflexivity", refl, refl_type, true),
        ("transport", transport, transport_type.clone(), true),
        (
            "pair",
            Term::pair(u.clone(), u.clone()),
            Term::sigma(Term::universe(1), Term::universe(1)),
            true,
        ),
        (
            "projection",
            Term::snd(Term::pair(u.clone(), u.clone())),
            Term::universe(1),
            true,
        ),
        (
            "wrong_path_origin",
            bind(
                &domains,
                Term::j(Term::var(1), Term::var(0), Term::refl(Term::var(3))),
                Term::lam,
            ),
            transport_type.clone(),
            false,
        ),
        (
            "invalid_motive",
            bind(
                &domains,
                Term::j(Term::var(0), Term::var(0), Term::var(2)),
                Term::lam,
            ),
            transport_type.clone(),
            false,
        ),
        (
            "invalid_base",
            bind(
                &domains,
                Term::j(Term::var(1), Term::var(3), Term::var(2)),
                Term::lam,
            ),
            transport_type,
            false,
        ),
        (
            "universe_overflow",
            Term::universe(u32::MAX),
            u.clone(),
            false,
        ),
    ];
    let distinct = [u, Term::var(0), Term::var(1)];
    cases.push((
        "distinct_endpoints",
        bind(&distinct, Term::refl(Term::var(1)), Term::lam),
        bind(
            &distinct,
            Term::id_ty(Term::var(2), Term::var(1), Term::var(0)),
            Term::pi,
        ),
        false,
    ));

    let rows: Vec<_> = cases
        .iter()
        .map(|(id, term, claimed_type, expected)| {
            let certificate = Certificate {
                term: term.clone(),
                claimed_type: claimed_type.clone(),
                metadata: Default::default(),
            };
            assert_eq!(certificate.verify().is_ok(), *expected, "native {id}");
            assert_eq!(
                verum_proof::proof_checker_nbe::verify_certificate(&certificate).is_ok(),
                *expected,
                "native NbE {id}"
            );
            serde_json::json!({"id": id, "term": term, "claimed_type": claimed_type})
        })
        .collect();
    let directory = std::env::temp_dir().join(format!("verum-lean-replay-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let battery = directory.join("certificates.json");
    std::fs::write(
        &battery,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = Command::new(&replay).arg(&battery).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = result["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (verdict, (id, _, _, expected)) in verdicts.iter().zip(&cases) {
        assert_eq!(verdict["id"], *id);
        assert_eq!(verdict["ok"], *expected, "Lean {id}: {verdict}");
    }
    // Decoder rejection must precede checking: neither the first constructor
    // key nor a valid subterm may conceal an additional authority.
    let mut malformed = serde_json::json!({"schema_version":1,"certificates":[rows[0].clone()]});
    malformed["certificates"][0]["term"]["Var"] = serde_json::json!(0);
    std::fs::write(&battery, serde_json::to_vec(&malformed).unwrap()).unwrap();
    let rejected = Command::new(&replay).arg(&battery).output().unwrap();
    assert_eq!(rejected.status.code(), Some(4));
    std::fs::remove_dir_all(directory).unwrap();
}
