use super::*;
use verum_proof::{limits::AdmissionLimits, proof_checker::Foundation};

fn source_judgment() -> (SourceBinding, Vec<u8>) {
    let source = "theorem identity(A:Type,x:A):A proof=x;";
    let report = replay(
        &serde_json::to_vec(&SourceRequest {
            profile: PROFILE.into(),
            source: source.into(),
            declaration: "identity".into(),
        })
        .unwrap(),
    );
    assert_eq!(report.status, "verified");
    (
        report.source,
        serde_json::to_vec(&report.judgment.unwrap()).unwrap(),
    )
}

#[test]
fn actual_native_resource_and_malformed_outcomes_preserve_source_binding() {
    let (source, bytes) = source_judgment();
    for limits in [
        AdmissionLimits {
            max_bytes: 0,
            ..AdmissionLimits::default()
        },
        AdmissionLimits {
            max_nodes: 0,
            ..AdmissionLimits::default()
        },
        AdmissionLimits {
            max_depth: 0,
            ..AdmissionLimits::default()
        },
    ] {
        let native = admission::replay_with_limits(&bytes, Foundation::Intensional, limits);
        assert_eq!(native.outcome, ReplayOutcome::ResourceExhausted);
        let report = native_refusal(native, &source);
        assert_eq!(report.status, "inconclusive");
        assert_eq!(report.source.sha256, source.sha256);
        assert!(report.judgment.is_none());
    }
    let report = native_refusal(admission::replay(b"{]"), &source);
    assert_eq!(report.status, "rejected");
    assert_eq!(report.diagnostic.unwrap().code, "invalid_certificate");
}

#[test]
fn typed_native_category_does_not_depend_on_a_diagnostic_code_whitelist() {
    let (source, bytes) = source_judgment();
    let mut native = admission::replay(&bytes);
    native.accepted = false;
    native.diagnostic = Some(Diagnostic {
        code: "future_resource_kind".into(),
        stage: "process".into(),
        message: "resource unavailable".into(),
    });
    for (outcome, status) in [
        (ReplayOutcome::ResourceExhausted, "inconclusive"),
        (ReplayOutcome::Inconclusive, "inconclusive"),
        (ReplayOutcome::Unsupported, "unsupported"),
        (ReplayOutcome::Rejected, "rejected"),
    ] {
        native.outcome = outcome;
        assert_eq!(native_refusal(native.clone(), &source).status, status);
    }
    native.outcome = ReplayOutcome::Accepted;
    let report = native_refusal(native, &source);
    assert_eq!(report.status, "inconclusive");
    assert_eq!(report.diagnostic.unwrap().code, "invalid_native_report");
}
