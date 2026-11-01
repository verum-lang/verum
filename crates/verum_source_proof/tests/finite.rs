use verum_proof::{admission, proof_checker::Term};
use verum_source_proof::{SourceReport, SourceRequest, finite, replay, sha256};

const TRANSITION: &str = include_str!("fixtures/finite_transition.vr");
fn check(source: &str, declaration: &str) -> SourceReport {
    replay(
        &serde_json::to_vec(&SourceRequest {
            profile: finite::PROFILE.into(),
            source: source.into(),
            declaration: declaration.into(),
        })
        .unwrap(),
    )
}
fn accepts(source: &str, declaration: &str) -> SourceReport {
    let report = check(source, declaration);
    assert_eq!(report.status, "verified", "{report:?}");
    let judgment = report.judgment.as_ref().unwrap();
    assert!(admission::replay(&serde_json::to_vec(judgment).unwrap()).accepted);
    report
}

#[test]
fn actual_transition_preserves_invariant_and_refines_reference_for_every_input() {
    let report = accepts(TRANSITION, "step");
    assert_eq!(report.source.sha256, sha256(TRANSITION.as_bytes()));
    let implementation = report.implementation.unwrap();
    assert_eq!(implementation.input_cases, 8);
    assert_eq!(implementation.requires_count, 1);
    assert_eq!(implementation.ensures_count, 2);
    assert_eq!(
        implementation
            .dependencies
            .iter()
            .map(|d| d.declaration.as_str())
            .collect::<Vec<_>>(),
        ["Phase", "admissible", "reference", "same_phase", "step"]
    );
    for dependency in implementation.dependencies {
        let span = dependency.span.unwrap();
        assert_eq!(
            span.sha256,
            sha256(TRANSITION[span.start as usize..span.end as usize].as_bytes())
        );
    }
}

#[test]
fn changing_only_the_actual_body_or_called_reference_fails_semantically() {
    let (prefix, body) = TRANSITION.split_once("fn step").unwrap();
    for changed in [
        format!(
            "{prefix}fn step{}",
            body.replace(
                "Phase.Waiting => Phase.Closing",
                "Phase.Waiting => Phase.Poisoned"
            )
        ),
        format!(
            "{prefix}fn step{}",
            body.replace("if !finished", "if finished")
        ),
        format!(
            "{}fn step{body}",
            prefix.replace(
                "Phase.Waiting => Phase.Closing",
                "Phase.Waiting => Phase.Published"
            )
        ),
    ] {
        let report = check(&changed, "step");
        assert_eq!(report.status, "rejected", "{report:?}");
        assert!(
            report.native.is_some(),
            "must fail in the kernel, not at parse time"
        );
        assert!(report.judgment.is_none());
    }
}

#[test]
fn immutable_scopes_boolean_operations_and_zero_argument_functions_are_exact() {
    accepts(
        "fn truth() -> Bool { true } fn t(x:Bool,y:Bool)->Bool ensures result == (x || y) { let x = x || y; let y = truth(); x && y }",
        "t",
    );
    accepts("fn t(x:Bool)->Bool requires x ensures result { x }", "t");
    accepts("fn t()->Bool ensures result { true }", "t");
    let bad = check(
        "fn t(x:Bool,y:Bool)->Bool ensures result == x { let x = y; x }",
        "t",
    );
    assert_eq!(bad.status, "rejected", "{bad:?}");
}

#[test]
fn unsupported_semantics_never_disappear_in_unused_arguments_or_branches() {
    for source in [
        "fn t(x:Bool)->Bool ensures result { if true { true } else { print(x) } }",
        "fn first(x:Bool,y:Bool)->Bool { x } fn t()->Bool ensures result { first(true, print(false)) }",
        "fn t(x:Bool)->Bool ensures result { t(x) }",
        "fn t(x:Bool)->Bool ensures result { let mut x = true; x }",
        "fn t(x:Int)->Bool ensures result { true }",
        "@custom fn t(x:Bool)->Bool ensures result { true }",
        "fn helper(x:Bool)->Bool requires x { x } fn t()->Bool ensures result { helper(false) }",
        "mount external.module; fn t()->Bool ensures result { true }",
        "type Bool is One | Two; fn t()->Bool ensures true { Bool.One }",
        "fn t(x:Bool)->Bool ensures result { match x { true => true } }",
        "fn t(x:Bool)->Bool ensures result { match x { true => true, true => true, false => true } }",
        "type Phase is Ready | Done; fn t(Phase:Phase)->Bool ensures result { match Phase { Phase.Ready => true, Phase.Done => true } }",
        "type A is Ready | Done; type B is Ready | Done; fn t(x:A)->Bool ensures result { match x { B.Ready => true, B.Done => true } }",
    ] {
        let report = check(source, "t");
        assert!(
            matches!(report.status.as_str(), "unsupported" | "rejected"),
            "{source}\n{report:?}"
        );
        assert!(report.judgment.is_none());
        assert!(report.implementation.is_none());
    }
}

#[test]
fn finite_profile_runs_through_the_bounded_source_worker() {
    let request = serde_json::to_vec(&SourceRequest {
        profile: finite::PROFILE.into(),
        source: TRANSITION.into(),
        declaration: "step".into(),
    })
    .unwrap();
    let output = verum_proof::process::run_worker(
        std::path::Path::new(env!("CARGO_BIN_EXE_verum-source-replay")),
        "--worker",
        &request,
    )
    .unwrap();
    let report: SourceReport = serde_json::from_slice(&output).unwrap();
    assert_eq!(report.status, "verified", "{report:?}");
    assert_eq!(report.profile, finite::PROFILE);
    assert_eq!(report.source.sha256, sha256(TRANSITION.as_bytes()));
    assert_eq!(report.implementation.unwrap().input_cases, 8);
}

#[test]
fn source_and_contract_changes_cannot_reuse_the_original_binding() {
    let original = accepts("fn t(x:Bool)->Bool ensures result == x { x }", "t");
    let changed = accepts("fn t(x:Bool)->Bool ensures true { !x }", "t");
    assert_ne!(original.source.sha256, changed.source.sha256);
    assert_ne!(
        original.judgment.unwrap().statement,
        changed.judgment.unwrap().statement
    );
    let report = check("fn t(x:Bool)->Bool { x }", "t");
    assert_eq!(report.status, "unsupported");
}

#[test]
#[ignore = "requires VERUM_EXECUTABLE pointing to the actual language executable"]
fn proved_source_executes_all_admitted_cases_in_the_actual_interpreter() {
    accepts(TRANSITION, "step");
    let executable = std::env::var_os("VERUM_EXECUTABLE").expect("build verum");
    let path = std::env::temp_dir().join(format!("verum-finite-runtime-{}.vr", std::process::id()));
    std::fs::write(&path, TRANSITION).unwrap();
    let output = std::process::Command::new(executable)
        .args(["run", "--interp", "--deny-all"])
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("finite-transition: all admitted cases passed")
    );
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_lean_replays_actual_function_and_rejects_corrupt_certificate() {
    let executable = std::env::var_os("VERUM_LEAN_REPLAY").expect("build independent Lean checker");
    let mut cases = Vec::new();
    for (source, name) in [
        (TRANSITION, "step"),
        (
            "fn t(x:Bool,y:Bool)->Bool ensures result == (x || y) { if x { true } else { y } }",
            "t",
        ),
    ] {
        let certificate = accepts(source, name).judgment.unwrap().certificate;
        cases.push((certificate.clone(), true));
        let mut corrupt = certificate;
        corrupt.term = Term::refl(finite::boolean());
        cases.push((corrupt, false));
    }
    let rows: Vec<_> = cases.iter().enumerate().map(|(id,(certificate,expected))| {
        assert_eq!(certificate.verify().is_ok(), *expected);
        assert_eq!(verum_proof::proof_checker_nbe::verify_certificate(certificate).is_ok(), *expected);
        serde_json::json!({"id":id.to_string(),"term":certificate.term,"claimed_type":certificate.claimed_type})
    }).collect();
    let path =
        std::env::temp_dir().join(format!("verum-source-finite-{}.json", std::process::id()));
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"certificates":rows})).unwrap(),
    )
    .unwrap();
    let output = std::process::Command::new(executable)
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
    assert_eq!(verdicts.len(), cases.len());
    for (id, (verdict, (_, expected))) in verdicts.iter().zip(cases).enumerate() {
        assert_eq!(verdict["id"], id.to_string());
        assert_eq!(verdict["ok"], expected, "{verdict}");
    }
}
