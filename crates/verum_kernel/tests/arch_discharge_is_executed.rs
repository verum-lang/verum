//! The architectural discharges are computed, not stamped (T0841).
//!
//! Every `kernel_arch_*` intrinsic used to answer `decision(true,
//! "<prose describing the check>")` — a verdict the kernel never
//! computed, minted before the ATS-V phase existed and left in place
//! after it went live. These gates keep that from coming back: a
//! discharge must be the result of running the checker it names, and
//! must go red the moment that checker stops working.

use verum_kernel::arch_anti_pattern::AntiPatternCode;
use verum_kernel::arch_probe::{ProbeOutcome, probe_table, run_all_probes, run_probe};
use verum_kernel::intrinsic_dispatch::{
    IntrinsicValue, available_intrinsics, dispatch_intrinsic, is_known_intrinsic,
};

/// Every architectural name the corpus can cite resolves. The eight
/// AP-033..AP-040 endpoints were cited by
/// `core/architecture/anti_patterns.vr` while absent from the
/// dispatch, and the discharge audit reported them as unrecognised —
/// a citation with no kernel behind it.
#[test]
fn every_cited_architectural_endpoint_resolves() {
    const CITED: &[&str] = &[
        "kernel_arch_retracted_citation_check",
        "kernel_arch_hypothesis_plan_check",
        "kernel_arch_interpretation_in_mature_check",
        "kernel_arch_observer_impersonation_check",
        "kernel_arch_boundless_audit_check",
        "kernel_arch_implicit_substrate_check",
        "kernel_arch_anchoring_overextension_check",
        "kernel_arch_self_reference_check",
    ];
    for name in CITED {
        assert!(
            is_known_intrinsic(name),
            "`{name}` is cited by core/architecture/anti_patterns.vr but the kernel \
             cannot dispatch it — the citation has no verifier behind it"
        );
        assert!(
            dispatch_intrinsic(name, &[]).is_some(),
            "`{name}` is registered but dispatches to nothing"
        );
    }
}

/// The registry and the dispatch agree by construction: every
/// architectural name the registry advertises actually dispatches.
/// Before the claim table became the single authority these were two
/// hand-maintained lists, and that is exactly how eight endpoints
/// went missing.
#[test]
fn registry_and_dispatch_cannot_drift() {
    for name in available_intrinsics().iter().filter(|n| n.starts_with("kernel_arch_")) {
        assert!(
            dispatch_intrinsic(name, &[]).is_some(),
            "`{name}` is advertised by available_intrinsics() but does not dispatch"
        );
    }
}

/// An architectural discharge reports what its probes found. This is
/// the gate against the stamp returning: it fails if a discharge
/// answers `true` while the checker it names is silent.
#[test]
fn a_discharge_reflects_its_probes() {
    // A single-pattern claim whose probe discharges must answer true…
    let value = dispatch_intrinsic("kernel_arch_cve_closure", &[])
        .expect("the CVE-closure discharge resolves");
    match value {
        IntrinsicValue::Decision { holds, reason } => {
            assert_eq!(
                run_probe(AntiPatternCode::CveIncomplete),
                Some(ProbeOutcome::Discharged),
                "precondition: the CVE-incomplete probe discharges"
            );
            assert!(holds, "the discharge must follow its probe; reason: {reason}");
            assert!(
                reason.contains("discharged by execution"),
                "the reason must say the verdict was EXECUTED, not asserted: {reason}"
            );
        }
        other => panic!("an architectural discharge must be a Decision, got {other:?}"),
    }

    // …and the whole-roster claim must name how many probes it ran,
    // so a reader can tell a real sweep from a stamp.
    let value = dispatch_intrinsic("kernel_arch_anti_pattern_check", &[])
        .expect("the catalogue discharge resolves");
    match value {
        IntrinsicValue::Decision { holds, reason } => {
            assert!(holds, "the catalogue discharge failed: {reason}");
            let roster = probe_table().len();
            assert!(
                reason.contains(&format!("{roster} probe(s)")),
                "the catalogue discharge must report the size of the sweep it ran \
                 ({roster} probes); reason: {reason}"
            );
        }
        other => panic!("expected a Decision, got {other:?}"),
    }
}

/// Nothing in the architectural family answers without evidence: each
/// dispatched name maps to at least one probe that actually ran.
#[test]
fn no_architectural_discharge_is_evidence_free() {
    let all_discharged = run_all_probes()
        .into_iter()
        .all(|(_, outcome)| outcome.is_discharged());
    assert!(
        all_discharged,
        "precondition: every roster probe discharges — otherwise the assertions below \
         would be measuring a broken checker rather than the discharge mechanism"
    );

    for name in available_intrinsics().iter().filter(|n| n.starts_with("kernel_arch_")) {
        match dispatch_intrinsic(name, &[]) {
            Some(IntrinsicValue::Decision { holds, reason }) => {
                assert!(holds, "`{name}` reports NOT discharged: {reason}");
                assert!(
                    reason.contains("probe(s) ran against the live checkers")
                        || reason.contains("structural claim DISCHARGED"),
                    "`{name}` answered without saying what it executed — the sanity-stamp \
                     shape this gate exists to prevent; reason: {reason}"
                );
            }
            other => panic!("`{name}` must answer with a Decision, got {other:?}"),
        }
    }
}

/// A name with no claim has no discharge. An unknown architectural
/// name must NOT be answered — the audit reports it as unrecognised,
/// which is visible, rather than as a pass, which is not.
#[test]
fn an_unclaimed_architectural_name_is_not_dischargeable() {
    assert!(
        dispatch_intrinsic("kernel_arch_no_such_property", &[]).is_none(),
        "an architectural name with no claim in the table must not dispatch — \
         answering it would be the stamp under a different spelling"
    );
    assert!(!is_known_intrinsic("kernel_arch_no_such_property"));
}
