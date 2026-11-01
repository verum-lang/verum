use verum_proof::{admission, proof_checker::Term};
use verum_source_proof::{SourceReport, SourceRequest, finite, replay, sha256};

const TRANSITION: &str = include_str!("fixtures/finite_transition.vr");
const CONTRACTED: &str = include_str!("fixtures/contracted_transition.vr");
const GUARDED: &str = "fn guarded(x:Bool)->Bool requires x ensures result { x }";
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
    for (source, declaration) in [(TRANSITION, "step"), (CONTRACTED, "publish")] {
        let request = serde_json::to_vec(&SourceRequest {
            profile: finite::PROFILE.into(),
            source: source.into(),
            declaration: declaration.into(),
        })
        .unwrap();
        let output = verum_proof::process::run_worker_with_limits(
            std::path::Path::new(env!("CARGO_BIN_EXE_verum-source-replay")),
            "--worker",
            &request,
            verum_source_proof::WORKER_LIMITS,
        )
        .unwrap();
        let report: SourceReport = serde_json::from_slice(&output).unwrap();
        assert_eq!(report.status, "verified", "{report:?}");
        assert_eq!(report.profile, finite::PROFILE);
        assert_eq!(report.source.sha256, sha256(source.as_bytes()));
        assert_eq!(report.implementation.unwrap().input_cases, 8);
        if declaration == "publish" {
            assert!(output.len() as u64 > verum_proof::process::OUTPUT_BYTES);
        }
    }
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
fn contracted_helpers_contribute_proofs_and_source_bound_call_obligations() {
    use finite::ObligationKind::{BodyCall, FunctionContract};
    let report = accepts(CONTRACTED, "publish");
    let bindings = report.implementation.as_ref().unwrap();
    assert_eq!(bindings.input_cases, 8);
    let kinds: Vec<_> = bindings
        .obligations
        .iter()
        .map(|o| (o.owner.declaration.as_str(), o.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("finish", FunctionContract),
            ("checked_finish", BodyCall),
            ("checked_finish", FunctionContract),
            ("publish", BodyCall),
            ("publish", FunctionContract)
        ]
    );
    for obligation in &bindings.obligations {
        if let Some(call) = &obligation.call {
            let text = &CONTRACTED[call.start as usize..call.end as usize];
            assert_eq!(call.sha256, sha256(text.as_bytes()));
            assert!(text.starts_with(&obligation.callee.as_ref().unwrap().declaration));
        }
    }
    let judgment = report.judgment.unwrap();
    assert!(matches!(judgment.statement, Term::Sigma(..)));
    // Removing a supporting proof cannot leave a certificate for the bundle.
    let mut missing = judgment.certificate;
    let Term::Pair(_, tail) = missing.term else {
        panic!("conjunctive proof bundle")
    };
    missing.term = *tail;
    assert!(missing.verify().is_err());
    assert!(verum_proof::proof_checker_nbe::verify_certificate(&missing).is_err());
}

#[test]
fn call_requirements_follow_branches_short_circuiting_and_local_scopes() {
    for function in [
        "fn t(x:Bool)->Bool requires x ensures result { guarded(x) }",
        "fn t(x:Bool)->Bool ensures result == x { if x { guarded(x) } else { false } }",
        "fn t(x:Bool)->Bool ensures result == x { x && guarded(x) }",
        "fn t(x:Bool)->Bool ensures result { !x || guarded(x) }",
        "fn t()->Bool ensures result { true || guarded(false) }",
        "fn t()->Bool ensures !result { false && guarded(false) }",
        "fn t(x:Bool)->Bool ensures result == x { match x { true => guarded(x), false => false } }",
        "fn t(x:Bool)->Bool ensures result == x { let y = x; if y { guarded(y) } else { false } }",
        "fn t(x:Bool)->Bool requires x, guarded(x) ensures result { x }",
        "fn t(x:Bool)->Bool requires x && guarded(x) ensures result { x }",
        "fn t(x:Bool)->Bool requires x ensures guarded(result) { x }",
        "fn t(x:Bool)->Bool ensures result { guarded(x || !x) }",
    ] {
        accepts(&format!("{GUARDED} {function}"), "t");
    }
    // A non-Boolean source carrier and a contract call in an ensures clause.
    let source = TRANSITION.replace("fn reference(phase: Phase, finished: Bool) -> Phase {",
        "fn reference(phase: Phase, finished: Bool) -> Phase requires admissible(phase) ensures admissible(result) {");
    let report = accepts(&source, "step");
    assert!(
        report
            .implementation
            .unwrap()
            .obligations
            .iter()
            .any(|o| o.kind == finite::ObligationKind::PostconditionCall)
    );
}

#[test]
fn true_postconditions_cannot_hide_undefined_calls_or_eager_arguments() {
    for function in [
        "fn t(x:Bool)->Bool ensures true { guarded(x) }",
        "fn t()->Bool ensures true { let unused = guarded(false); true }",
        "fn ignore(x:Bool,y:Bool)->Bool { x } fn t()->Bool ensures true { ignore(true,guarded(false)) }",
        "fn t()->Bool ensures true { guarded(false) == guarded(false) }",
        "fn t()->Bool ensures true { if guarded(false) { true } else { true } }",
        "fn t()->Bool ensures true { true && guarded(false) }",
        "fn t()->Bool ensures true { false || guarded(false) }",
        "fn t()->Bool ensures true { let x = false; let x = true; guarded(!x) }",
        "fn t(x:Bool)->Bool ensures true { match x { true => true, false => guarded(x) } }",
        "fn wrapper(x:Bool)->Bool { guarded(x) } fn t()->Bool ensures true { wrapper(true) }",
    ] {
        let source = format!("{GUARDED} {function}");
        let report = check(&source, "t");
        assert_eq!(report.status, "rejected", "{source}\n{report:?}");
        assert!(
            report.native.is_some(),
            "semantic kernel rejection required"
        );
        let span = report.diagnostic_span.unwrap();
        assert!(source[span.start as usize..span.end as usize].starts_with("guarded("));
        assert!(report.diagnostic.unwrap().message.contains("BodyCall"));
        assert!(report.judgment.is_none());
    }
}

#[test]
fn contracts_cannot_justify_their_own_evaluation_or_assume_helper_promises() {
    for (function, phase) in [
        (
            "fn t(x:Bool)->Bool requires guarded(x) ensures true { true }",
            "PreconditionCall",
        ),
        (
            "fn t(x:Bool)->Bool requires guarded(x), x ensures true { true }",
            "PreconditionCall",
        ),
        (
            "fn t(x:Bool)->Bool ensures guarded(result) == result { x }",
            "PostconditionCall",
        ),
    ] {
        let report = check(&format!("{GUARDED} {function}"), "t");
        assert_eq!(report.status, "rejected", "{report:?}");
        assert!(report.native.is_some());
        assert!(report.diagnostic.unwrap().message.contains(phase));
    }
    let source =
        "fn lie(x:Bool)->Bool ensures result == x { !x } fn t()->Bool ensures true { lie(true) }";
    let report = check(source, "t");
    assert_eq!(report.status, "rejected", "{report:?}");
    assert!(
        report
            .diagnostic
            .unwrap()
            .message
            .contains("FunctionContract obligation in lie")
    );
    let span = report.diagnostic_span.unwrap();
    assert!(source[span.start as usize..span.end as usize].starts_with("fn lie"));
}

#[test]
fn circular_specs_and_compositional_resource_exhaustion_do_not_verify() {
    for source in [
        "fn t(x:Bool)->Bool ensures result == t(x) { x }",
        "fn a(x:Bool)->Bool ensures b(x) { x } fn b(x:Bool)->Bool ensures a(x) { x } fn t()->Bool ensures true { a(true) }",
        "fn t(x:Bool)->Bool requires t(x) ensures true { true }",
    ] {
        let report = check(source, "t");
        assert_eq!(report.status, "unsupported", "{report:?}");
        assert!(report.judgment.is_none());
    }
    let params = (0..10)
        .map(|n| format!("x{n}:Bool"))
        .collect::<Vec<_>>()
        .join(",");
    let locals = (0..17)
        .map(|n| format!("let v{n} = guarded(true);"))
        .collect::<String>();
    let report = check(
        &format!("{GUARDED} fn t({params})->Bool ensures result {{ {locals} true }}"),
        "t",
    );
    assert_eq!(report.status, "inconclusive", "{report:?}");
    assert_eq!(report.diagnostic.unwrap().code, "finite_case_limit");
}

#[test]
#[ignore = "requires VERUM_EXECUTABLE pointing to the actual language executable"]
fn proved_source_executes_all_admitted_cases_in_the_actual_interpreter() {
    let executable = std::env::var_os("VERUM_EXECUTABLE").expect("build verum");
    for (source, declaration, marker) in [
        (
            TRANSITION,
            "step",
            "finite-transition: all admitted cases passed",
        ),
        (
            CONTRACTED,
            "publish",
            "contracted-transition: all input cases passed",
        ),
    ] {
        accepts(source, declaration);
        let path = std::env::temp_dir().join(format!(
            "verum-finite-runtime-{declaration}-{}.vr",
            std::process::id()
        ));
        std::fs::write(&path, source).unwrap();
        let output = std::process::Command::new(&executable)
            .args(["run", "--interp", "--deny-all"])
            .arg(&path)
            .output()
            .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{declaration}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(marker));
    }
}

#[test]
#[ignore = "requires VERUM_LEAN_REPLAY pointing to the independent checker"]
fn independent_lean_replays_actual_function_and_rejects_corrupt_certificate() {
    let executable = std::env::var_os("VERUM_LEAN_REPLAY").expect("build independent Lean checker");
    let mut cases = Vec::new();
    for (source, name) in [
        (TRANSITION, "step"),
        (CONTRACTED, "publish"),
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
    // Invalid source still yields an unaccepted candidate from elaboration.
    // Replay that exact candidate independently, rather than counting a source
    // admission refusal as a Lean semantic verdict.
    for source in [
        "fn h(x:Bool)->Bool requires x ensures result { x } fn t(x:Bool)->Bool ensures true { h(x) }",
        "fn h(x:Bool)->Bool ensures result == x { !x } fn t()->Bool ensures true { h(true) }",
        "fn h(x:Bool)->Bool requires x ensures result { x } fn t(x:Bool)->Bool requires h(x) ensures true { true }",
    ] {
        let module = verum_fast_parser::FastParser::new()
            .parse_module_str(source, verum_common::span::FileId::new(0))
            .unwrap();
        cases.push((
            finite::elaborate(source, &module, "t").unwrap().certificate,
            false,
        ));
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
