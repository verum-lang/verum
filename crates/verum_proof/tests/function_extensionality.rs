#[path = "support/function_extensionality.rs"]
mod fixtures;

use verum_proof::{
    admission,
    proof_checker::{Foundation, Term},
    proof_checker_nbe as nbe,
};

#[test]
fn dependent_extensionality_requires_exact_foundation_and_all_premises() {
    for (name, cert, valid) in fixtures::battery() {
        for foundation in [Foundation::Intensional, Foundation::FunctionExtensionality] {
            let expected = valid && foundation == Foundation::FunctionExtensionality;
            let substitution = cert.verify_with_foundation(foundation);
            let normalization = nbe::verify_certificate_with_foundation(&cert, foundation);
            assert_eq!(
                substitution.is_ok(),
                expected,
                "{name}: {foundation:?}: {substitution:?}"
            );
            assert_eq!(
                normalization.is_ok(),
                expected,
                "{name}: {foundation:?}: {normalization:?}"
            );
            let bytes = serde_json::to_vec(&admission::Judgment {
                profile: admission::profile_for(foundation).into(),
                statement: cert.claimed_type.clone(),
                certificate: cert.clone(),
            })
            .unwrap();
            let report = admission::replay_with_foundation(&bytes, foundation);
            assert_eq!(report.accepted, expected, "{name}: {report:?}");
            assert_eq!(report.profile, admission::profile_for(foundation));
        }
    }
}

#[test]
fn artifact_metadata_or_profile_cannot_strengthen_the_verifier() {
    let mut cert = fixtures::general();
    cert.metadata
        .insert("foundation".into(), "function-extensionality".into());
    assert!(cert.verify().is_err());
    assert!(nbe::verify_certificate(&cert).is_err());
    let bytes = serde_json::to_vec(&admission::Judgment {
        profile: admission::EXTENSIONAL_PROFILE.into(),
        statement: cert.claimed_type.clone(),
        certificate: cert,
    })
    .unwrap();
    let refused = admission::replay(&bytes);
    assert!(!refused.accepted);
    assert_eq!(refused.diagnostic.unwrap().code, "unsupported_profile");
}

#[cfg(feature = "process")]
#[test]
fn isolated_worker_uses_the_verifiers_foundation() {
    let cert = fixtures::general();
    let bytes = serde_json::to_vec(&admission::Judgment {
        profile: admission::EXTENSIONAL_PROFILE.into(),
        statement: cert.claimed_type.clone(),
        certificate: cert,
    })
    .unwrap();
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_verum-proof"));
    let report = verum_proof::process::replay_with_foundation(
        executable,
        &bytes,
        Foundation::FunctionExtensionality,
    );
    assert!(report.accepted, "{report:?}");
    assert_eq!(report.profile, admission::EXTENSIONAL_PROFILE);
    assert!(!verum_proof::process::replay(executable, &bytes).accepted);
}

#[test]
fn extensionality_proofs_remain_opaque_even_for_reflexive_endpoints() {
    let ty = Term::pi(Term::universe(0), Term::universe(0));
    let function = Term::lam(Term::universe(0), Term::var(0));
    let proof = Term::fun_ext(
        ty,
        function.clone(),
        function.clone(),
        Term::lam(Term::universe(0), Term::refl(Term::var(0))),
    );
    let value = nbe::eval(&proof, &vec![]);
    assert!(!nbe::def_eq(
        &value,
        &nbe::eval(&Term::refl(function.clone()), &vec![]),
        0
    ));
    assert_eq!(nbe::quote(&value, 0), proof);
    let identity = Term::id_ty(
        Term::pi(Term::universe(0), Term::universe(0)),
        function.clone(),
        function.clone(),
    );
    let forged = verum_proof::proof_checker::Certificate {
        term: Term::refl(proof.clone()),
        claimed_type: Term::id_ty(identity, proof, Term::refl(function)),
        metadata: Default::default(),
    };
    assert!(
        forged
            .verify_with_foundation(Foundation::FunctionExtensionality)
            .is_err()
    );
    assert!(
        nbe::verify_certificate_with_foundation(&forged, Foundation::FunctionExtensionality)
            .is_err()
    );
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent built checker"]
fn independent_lean_agrees_on_foundations_and_dependent_premises() {
    let mut rows = Vec::new();
    let mut expected = Vec::new();
    for (name, cert, valid) in fixtures::battery() {
        for foundation in [Foundation::Intensional, Foundation::FunctionExtensionality] {
            rows.push(serde_json::json!({"id":format!("{name}-{foundation:?}"),
                "profile":admission::profile_for(foundation), "term":cert.term, "claimed_type":cert.claimed_type}));
            expected.push(valid && foundation == Foundation::FunctionExtensionality);
        }
    }
    let path = std::env::temp_dir().join(format!("verum-funext-{}.json", std::process::id()));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(std::env::var_os("VERUM_LEAN_REPLAY").unwrap())
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let verdicts = result["verdicts"].as_array().unwrap();
    assert_eq!(verdicts.len(), expected.len());
    for ((row, expected), verdict) in rows.iter().zip(expected).zip(verdicts) {
        assert_eq!(verdict["id"], row["id"]);
        assert_eq!(verdict["ok"], expected, "{verdict}");
    }
}
