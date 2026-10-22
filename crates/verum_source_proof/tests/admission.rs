use std::{path::Path, time::Duration};
use verum_proof::{process, proof_checker::Term};
use verum_source_proof::{PROFILE, SourceReport, SourceRequest, replay, sha256};
fn request(source: &str, declaration: &str) -> Vec<u8> {
    serde_json::to_vec(&SourceRequest {
        profile: PROFILE.into(),
        source: source.into(),
        declaration: declaration.into(),
    })
    .unwrap()
}
fn check(source: &str, declaration: &str) -> SourceReport {
    replay(&request(source, declaration))
}
const ID: &str = "// UTF-8: доказательство\npublic theorem identity(a:Type, x:a): a proof = x;\n";
#[test]
fn exact_source_span_and_complete_assumptions_reconstruct_the_statement() {
    let report = check(ID, "identity");
    assert_eq!(report.status, "verified", "{report:?}");
    assert_eq!(report.source.sha256, sha256(ID.as_bytes()));
    let span = report.source.span.unwrap();
    let exact = &ID[span.start as usize..span.end as usize];
    assert_eq!(exact, "public theorem identity(a:Type, x:a): a proof = x;");
    assert_eq!(span.sha256, sha256(exact.as_bytes()));
    let judgment = report.judgment.unwrap();
    let rebuilt = report
        .telescope
        .iter()
        .rev()
        .fold(report.conclusion.unwrap(), |body, b| {
            Term::pi(b.domain.clone(), body)
        });
    assert_eq!(rebuilt, judgment.statement);
    let mut witness = &judgment.certificate.term;
    for b in report.telescope {
        let Term::Lam(domain, body) = witness else {
            panic!("missing binder")
        };
        assert_eq!(**domain, b.domain);
        witness = body;
    }
    assert!(report.native.unwrap().accepted);
}
#[test]
fn source_goal_proof_and_names_are_not_interchangeable() {
    for text in [
        ID.replace("proof = x", "proof = a"),
        ID.replace("proof = x", "proof = missing"),
        ID.replace(": a proof", ": x proof"),
        ID.replace("x:a", "x:missing"),
    ] {
        assert_ne!(check(&text, "identity").status, "verified", "{text}");
    }
    let renamed = ID.replace("identity", "identity2");
    assert_eq!(
        check(&renamed, "identity").diagnostic.unwrap().code,
        "declaration_not_found"
    );
    let extra = format!("{ID}{ID}");
    assert_eq!(
        check(&extra, "identity").diagnostic.unwrap().code,
        "ambiguous_declaration"
    );
}
#[test]
fn neither_attributes_contracts_nor_runtime_bodies_are_silently_erased() {
    for text in [
        "@custom public theorem t(a:Type,x:a):a proof=x;",
        "theorem t(a:Type,x:a) @custom ensures a proof=x;",
        "theorem t(a:Type,b:Type,x:a) ensures b : a proof=x;",
        "theorem t(a:Type,x:a):a proof { show a by assumption; }",
        "fn t(a:Type,x:a)->a {x}",
    ] {
        assert_ne!(check(text, "t").status, "verified", "{text}");
    }
}
#[test]
fn dependent_families_preserve_arguments_and_reject_a_wrong_fiber() {
    let text = "theorem fiber(a:Type,p:fn(a)->Type,x:a,y:a,h:p(x)):p(x) proof=h;";
    assert_eq!(check(text, "fiber").status, "verified");
    let wrong = text.replace("):p(x) proof", "):p(y) proof");
    assert_eq!(check(&wrong, "fiber").status, "rejected");
    let wrong_head = text.replace("p:fn(a)->Type", "p:a");
    assert_eq!(check(&wrong_head, "fiber").status, "rejected");
}
#[test]
fn rank_two_families_keep_explicit_meta_domains_and_capture() {
    let text = "theorem apply_family(a:Type,x:a,h:fn<p:meta a>(Path<a>(p,p))->a,eq:Path<a>(x,x)):a proof=h(x,eq);";
    assert_eq!(
        check(text, "apply_family").status,
        "verified",
        "{:?}",
        check(text, "apply_family")
    );
    let wrong = text.replace("eq:Path<a>(x,x)", "eq:a");
    assert_eq!(check(&wrong, "apply_family").status, "rejected");
}
#[test]
fn strict_envelope_and_resource_failures_never_become_certificates() {
    let text = String::from_utf8(request(ID, "identity")).unwrap();
    for malformed in [
        text.replacen('{', "{\"profile\":\"verum-source-pi-id/1\",", 1),
        text.replacen('{', "{\"accepted\":true,", 1),
        "null".into(),
    ] {
        let report = replay(malformed.as_bytes());
        assert_eq!(report.status, "rejected");
        assert!(report.judgment.is_none());
    }
    let report = replay(&vec![b' '; verum_proof::admission::MAX_BYTES + 1]);
    assert_eq!(report.status, "inconclusive");
    assert_eq!(report.diagnostic.unwrap().code, "byte_limit");
}
#[test]
fn real_source_worker_uses_the_shared_bounded_supervisor() {
    let executable = Path::new(env!("CARGO_BIN_EXE_verum-source-replay"));
    let bytes = process::run_worker(executable, "--worker", &request(ID, "identity")).unwrap();
    let report: SourceReport = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report.status, "verified");
    let native = serde_json::to_vec(&report.judgment.unwrap()).unwrap();
    assert!(verum_proof::admission::replay(&native).accepted);
    assert_eq!(
        process::run_worker_with_deadline(
            executable,
            "--worker",
            &request(ID, "identity"),
            Duration::ZERO
        )
        .unwrap_err()
        .code,
        "wall_limit"
    );
    assert_eq!(
        process::run_worker(
            Path::new("/nonexistent/verum-source-replay"),
            "--worker",
            b"{}"
        )
        .unwrap_err()
        .code,
        "worker_unavailable"
    );
}

#[test]
fn other_declarations_and_imports_do_not_add_implicit_assumptions() {
    let source = "mount untrusted.module; axiom arbitrary(a:Type) ensures a; theorem t(a:Type,x:a):a proof=x;";
    let valid = check(source, "t");
    assert_eq!(valid.status, "verified");
    assert_eq!(valid.telescope.len(), 2);
    assert_eq!(
        check(&source.replace("proof=x", "proof=arbitrary"), "t").status,
        "rejected"
    );
    assert_eq!(
        check(&source.replace("proof=x", "proof=arbitrary(a)"), "t").status,
        "rejected"
    );
}
