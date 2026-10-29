use verum_common::List;
use verum_proof::proof_checker::{Certificate, Term};
use verum_source_proof::{PROFILE, SourceReport, SourceRequest, replay, sha256};

fn check(source: &str, declaration: &str) -> SourceReport {
    replay(
        &serde_json::to_vec(&SourceRequest {
            profile: PROFILE.into(),
            source: source.into(),
            declaration: declaration.into(),
        })
        .unwrap(),
    )
}

fn accepts(source: &str, declaration: &str) -> Certificate {
    let report = check(source, declaration);
    assert_eq!(report.status, "verified", "{source}\n{report:?}");
    assert_eq!(report.source.sha256, sha256(source.as_bytes()));
    let judgment = report.judgment.unwrap();
    assert!(verum_proof::admission::replay(&serde_json::to_vec(&judgment).unwrap()).accepted);
    judgment.certificate
}

const TRANSPORT: &str = include_str!("fixtures/transport.vr");
const SYMMETRY: &str =
    "theorem t(a:Type,x:a,y:a,h:Path<a>(x,y)):y==x proof by rewrite(h); reflexivity;";
const CHAIN: &str = "theorem t(a:Type,x:a,y:a,z:a,h:Path<a>(x,y),k:Path<a>(y,z)):x==z proof by { rewrite(h); { rewrite(k); reflexivity; } };";
const CONGRUENCE: &str = "theorem t(a:Type,b:Type,f:fn(a)->b,x:a,y:a,h:Path<a>(x,y)):f(x)==f(y) proof by rewrite(h); reflexivity;";
const INTRO: &str = "theorem t(a:Type,x:a):forall y:a. forall (h:Path<a>(x,y)). y==x proof by intro(value,equality); rewrite(equality); reflexivity;";
const FAMILY: &str = "theorem t(a:Type,p:fn(a)->Type,x:a,y:a,h:Path<a>(x,y)):forall (z:p(x)). p(y) proof by rewrite(h); intro(value); exact(value);";

fn proof_body(term: &mut Term) -> &mut Term {
    match term {
        Term::Lam(_, body) => proof_body(body),
        _ => term,
    }
}

#[test]
fn transport_has_the_exact_dependent_statement_and_kernel_term() {
    let cert = accepts(TRANSPORT, "transport");
    // Independent de Bruijn oracle: A, P:A->Type, x:A, y:A,
    // equality:Id(A,x,y), value:P(x). The conclusion is P(y).
    let domains = [
        Term::universe(0),
        Term::pi(Term::var(0), Term::universe(0)),
        Term::var(1),
        Term::var(2),
        Term::id_ty(Term::var(3), Term::var(1), Term::var(0)),
        Term::app(Term::var(3), Term::var(2)),
    ];
    let expected = domains
        .iter()
        .rev()
        .fold(Term::app(Term::var(4), Term::var(2)), |body, domain| {
            Term::pi(domain.clone(), body)
        });
    assert_eq!(cert.claimed_type, expected);
    let mut body = &cert.term;
    for _ in domains {
        let Term::Lam(_, next) = body else {
            panic!("missing binder")
        };
        body = next;
    }
    assert_eq!(
        *body,
        Term::j(
            Term::lam(Term::var(5), Term::app(Term::var(5), Term::var(0))),
            Term::var(0),
            Term::var(1)
        )
    );
}

#[test]
fn equality_laws_are_derived_without_axioms_or_theorem_dispatch() {
    for source in [SYMMETRY, CHAIN, CONGRUENCE, INTRO, FAMILY] {
        accepts(source, "t");
    }
    accepts(
        "theorem t<universe u,universe v>(a:Type(u),p:fn(a)->Type(v),x:a,y:a,h:Path<a>(x,y),value:p(y)):p(x) proof by rewrite(h); assumption;",
        "t",
    );
    // Rewrite a compound endpoint, including both copies in the conclusion.
    accepts(
        "theorem t(a:Type,f:fn(a)->a,x:a,y:a,h:Path<a>(f(x),y)):f(x)==y proof by rewrite(h); reflexivity;",
        "t",
    );
}

#[test]
fn introductions_and_rewrites_preserve_shadowing_and_alpha_renaming() {
    let shadowed = "theorem t(a:Type,x:a):forall x:a. forall y:a. forall (h:Path<a>(x,y)). y==x proof by intro(x,y,h); rewrite(h); reflexivity;";
    let renamed = "theorem t(a:Type,outer:a):forall left:a. forall right:a. forall (eq:Path<a>(left,right)). right==left proof by intro(first,second,path); rewrite(path); reflexivity;";
    let a = accepts(shadowed, "t");
    let b = accepts(renamed, "t");
    assert_eq!(a.term, b.term);
    assert_eq!(a.claimed_type, b.claimed_type);
    accepts(
        "theorem t(a:Type):forall x:a. a proof by intro; assumption;",
        "t",
    );
    let report = check(
        "theorem t(a:Type,b:Type,x:a):forall x:b. a proof by intro(x); exact(x);",
        "t",
    );
    assert_eq!(report.status, "rejected");
}

#[test]
fn invalid_proofs_do_not_become_successful_goal_mutations() {
    for source in [
        "theorem t(a:Type,x:a,y:a):x==y proof by reflexivity;",
        "theorem t(a:Type,x:a,y:a):x==y proof by rewrite(x); reflexivity;",
        "theorem t(a:Type,x:a,y:a,z:a,h:Path<a>(x,y)):x==z proof by rewrite(h); reflexivity;",
        "theorem t(a:Type,x:a,y:a,z:a,h:Path<a>(x,y)):z==z proof by rewrite(h); reflexivity;",
        "theorem t(a:Type,x:a,y:a,h:Path<a>(x,y)):x==y proof by rewrite(h);",
        "theorem t(a:Type,x:a):x==x proof by reflexivity; exact(x);",
        "theorem t(a:Type,x:a):x==x proof by {};",
        "theorem t(a:Type,x:a):forall y:a. a proof by intro(y);",
    ] {
        let report = check(source, "t");
        assert_eq!(report.status, "rejected", "{source}\n{report:?}");
        assert!(report.judgment.is_none());
    }
    let wrong_fibre = TRANSPORT.replace("value: p(x)", "value: p(y)");
    assert_eq!(check(&wrong_fibre, "transport").status, "rejected");
}

#[test]
fn a_rewrite_cannot_retype_existing_dependent_values() {
    // Replacing x with a fresh endpoint changes f's argument domain, while
    // z still has type p(x). Even though the final equality is reflexive,
    // the family needed for transport is ill-typed and must be refused.
    let source = "theorem t(a:Type,p:fn(a)->Type,x:a,y:a,h:Path<a>(x,y),z:p(x),f:fn<t:meta a>(p(t))->a):f(x,z)==f(x,z) proof by rewrite(h); reflexivity;";
    let report = check(source, "t");
    assert_eq!(report.status, "rejected", "{report:?}");
    assert!(report.judgment.is_none());
    let span = report.diagnostic_span.unwrap();
    assert_eq!(&source[span.start as usize..span.end as usize], "(h)");
}

#[test]
fn unsupported_context_rewriting_is_explicit_and_tactic_error_spans_are_exact() {
    let source =
        "theorem t(a:Type,x:a,y:a,h:Path<a>(x,y)):x==y proof by rewrite(h) at h; reflexivity;";
    assert_eq!(check(source, "t").status, "unsupported");
    let wrong = TRANSPORT.replace("exact(value)", "exact(x)");
    let report = check(&wrong, "transport");
    assert_eq!(report.status, "rejected");
    let span = report.diagnostic_span.unwrap();
    assert_eq!(&wrong[span.start as usize..span.end as usize], "(x)");
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independently_replay_source_transports_and_corrupted_certificates() {
    let replay = std::env::var_os("VERUM_LEAN_REPLAY").expect("build verum_replay_checker");
    let mut cases = List::new();
    for (source, name) in [
        (TRANSPORT, "transport"),
        (SYMMETRY, "t"),
        (CHAIN, "t"),
        (CONGRUENCE, "t"),
        (INTRO, "t"),
        (FAMILY, "t"),
    ] {
        let certificate = accepts(source, name);
        cases.push((certificate.clone(), true));
        // Retain the complete scope and statement but forge reflexivity in
        // place of transport. All original goals have unequal endpoints or
        // an abstract family: a well-typed reflexivity term is insufficient.
        let mut corrupt = certificate.clone();
        *proof_body(&mut corrupt.term) = Term::refl(Term::var(0));
        cases.push((corrupt, false));
    }
    for mutation in 0..3 {
        let mut cert = accepts(TRANSPORT, "transport");
        let Term::J {
            motive,
            base,
            scrutinee,
        } = proof_body(&mut cert.term)
        else {
            panic!("transport constructor")
        };
        match mutation {
            0 => **scrutinee = Term::refl(Term::var(2)), // y=y cannot transport P(x).
            1 => **base = Term::var(2),                  // y:A is not evidence of P(x).
            2 => **motive = Term::lam(Term::var(5), Term::var(6)), // constant A is not P.
            _ => unreachable!(),
        }
        cases.push((cert, false));
    }
    let rows: List<_> = cases.iter().enumerate().map(|(id,(cert,expected))| {
        assert_eq!(cert.verify().is_ok(), *expected);
        assert_eq!(verum_proof::proof_checker_nbe::verify_certificate(cert).is_ok(), *expected);
        serde_json::json!({"id":id.to_string(),"term":cert.term,"claimed_type":cert.claimed_type})
    }).collect();
    let path =
        std::env::temp_dir().join(format!("verum-source-equality-{}.json", std::process::id()));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(replay)
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = report["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), cases.len());
    for (id, (verdict, (_, expected))) in verdicts.iter().zip(cases).enumerate() {
        assert_eq!(verdict["id"], id.to_string());
        assert_eq!(verdict["ok"], expected, "{verdict}");
    }
}
