use super::*;
use crate::admission::{self, Judgment};
use crate::proof_checker::{Certificate, Term};

fn response() -> (Report, ReplayLimits, InstalledLimits) {
    let limits = ReplayLimits::default();
    let installed = InstalledLimits {
        cpu_seconds: 2,
        address_space_bytes: None,
        stack_bytes: 8 * 1024 * 1024,
        output_bytes: limits.process.output_bytes,
    };
    let bytes = serde_json::to_vec(&Judgment {
        profile: admission::PROFILE.into(),
        statement: Term::universe(1),
        certificate: Certificate {
            term: Term::universe(0),
            claimed_type: Term::universe(1),
            metadata: Default::default(),
        },
    })
    .unwrap();
    let mut report = admission::replay(&bytes);
    report.worker = Some(WorkerReceipt {
        requested: limits,
        installed: Some(installed),
    });
    (report, limits, installed)
}

#[test]
fn worker_receipt_checks_requested_and_installed_policy_and_outcome() {
    let (report, limits, installed) = response();
    let check = |report: &Report| {
        validate_worker_report(
            &serde_json::to_vec(report).unwrap(),
            Foundation::Intensional,
            limits,
            installed,
        )
    };
    assert_eq!(check(&report).unwrap(), report);
    let mut altered = report.clone();
    altered
        .worker
        .as_mut()
        .unwrap()
        .requested
        .process
        .wall_time_ms -= 1;
    assert_eq!(
        check(&altered).unwrap_err().outcome,
        ReplayOutcome::Inconclusive
    );
    let mut altered = report.clone();
    altered
        .worker
        .as_mut()
        .unwrap()
        .installed
        .as_mut()
        .unwrap()
        .cpu_seconds -= 1;
    assert_eq!(
        check(&altered).unwrap_err().diagnostic.code,
        "worker_protocol"
    );
    let mut altered = report.clone();
    altered.limits.max_nodes -= 1;
    assert!(check(&altered).is_err());
    let mut altered = report.clone();
    altered.outcome = ReplayOutcome::ResourceExhausted;
    assert!(check(&altered).is_err());
    let mut altered = report.clone();
    altered.checker = "another checker".into();
    assert!(check(&altered).is_err());
    let mut altered = report.clone();
    altered.profile = admission::EXTENSIONAL_PROFILE.into();
    assert!(check(&altered).is_err());
    let mut altered = report;
    altered.worker.as_mut().unwrap().installed = None;
    assert!(check(&altered).is_err());
}

#[test]
fn missing_installed_receipt_is_only_valid_for_an_actual_sandbox_refusal() {
    let (_, limits, installed) = response();
    let mut report = report_failure(
        Foundation::Intensional,
        limits,
        inconclusive("sandbox_unavailable", "installation failed"),
    );
    assert!(
        validate_worker_report(
            &serde_json::to_vec(&report).unwrap(),
            Foundation::Intensional,
            limits,
            installed
        )
        .is_ok()
    );
    report.diagnostic.as_mut().unwrap().code = "worker_failed".into();
    assert!(
        validate_worker_report(
            &serde_json::to_vec(&report).unwrap(),
            Foundation::Intensional,
            limits,
            installed
        )
        .is_err()
    );
}
