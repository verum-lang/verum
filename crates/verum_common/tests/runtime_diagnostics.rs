//! The transport contract is shared by runtime producers and Studio consumers.
//! Its identity checks must not depend on a compiler or a JSON reader being present.
use verum_common::{
    CbgrViolationKind, List,
    runtime_diagnostics::{
        CbgrEvent, CbgrReport, CbgrSchema, CbgrTargetKind, RuntimeObservation, RuntimeReport,
        RuntimeReportSchema,
    },
};

fn report() -> RuntimeReport {
    RuntimeReport {
        schema: RuntimeReportSchema::V1,
        run_id: "current-run".into(),
        source_sha256: "a".repeat(64).into(),
        prepared_source_sha256: "b".repeat(64).into(),
        wrapper_lines: 1,
        entry_source_path: None,
        compiler_version: "0.1.0".into(),
        observation: RuntimeObservation::Captured,
        cbgr: Some(CbgrReport {
            schema: CbgrSchema::DispatchV1,
            capacity: 1,
            checks_enabled_at_start: true,
            truncated: false,
            events: [CbgrEvent {
                kind: CbgrViolationKind::GenerationMismatch,
                target_kind: CbgrTargetKind::Register,
                target: u32::MAX.to_string().into(),
                expected: Some(u64::MAX.to_string().into()),
                actual: Some("0".into()),
                function_id: Some(0),
                function: Some("main".into()),
                pc: 7,
                opcode: 20,
                source: None,
            }]
            .into_iter()
            .collect(),
        }),
    }
}

#[test]
fn identical_source_does_not_authorize_another_runs_observations() {
    let report = report();
    let source = "a".repeat(64);
    assert!(report.validate_binding("current-run", &source).is_ok());
    assert!(report.validate_binding("previous-run", &source).is_err());
    assert!(
        report
            .validate_binding("current-run", &"c".repeat(64))
            .is_err()
    );
}

#[test]
fn missing_capture_cannot_be_relabelled_as_an_observed_clean_run() {
    let source = "a".repeat(64);
    let mut report = report();
    report.observation = RuntimeObservation::Unavailable;
    assert!(report.validate_binding("current-run", &source).is_err());
    report.cbgr = None;
    assert!(report.validate_binding("current-run", &source).is_ok());
    report.observation = RuntimeObservation::Captured;
    assert!(report.validate_binding("current-run", &source).is_err());
}

#[test]
fn counters_keep_their_full_width_and_target_kind() {
    let source = "a".repeat(64);
    let mut report = report();
    assert!(report.validate_binding("current-run", &source).is_ok());
    report.cbgr.as_mut().unwrap().events[0].target = "4294967296".into();
    assert!(report.validate_binding("current-run", &source).is_err());
    let event = &mut report.cbgr.as_mut().unwrap().events[0];
    event.target_kind = CbgrTargetKind::Address;
    event.target = "0xffffffffffffffff".into();
    assert!(report.validate_binding("current-run", &source).is_ok());
    report.cbgr.as_mut().unwrap().events[0].actual = Some("18446744073709551616".into());
    assert!(report.validate_binding("current-run", &source).is_err());
}

#[test]
fn exhausted_capture_preserves_loss_without_accepting_out_of_bounds_events() {
    let source = "a".repeat(64);
    let mut report = report();
    report.cbgr.as_mut().unwrap().capacity = 0;
    assert!(report.validate_binding("current-run", &source).is_err());
    let capture = report.cbgr.as_mut().unwrap();
    capture.events.clear();
    capture.truncated = true;
    assert!(report.validate_binding("current-run", &source).is_ok());
    report.cbgr.as_mut().unwrap().capacity = 257;
    assert!(report.validate_binding("current-run", &source).is_err());
}

#[test]
fn failed_reservation_preserves_the_existing_list() {
    let mut values = List::new();
    values.push(7u64);
    assert!(values.try_reserve_exact(usize::MAX).is_err());
    assert_eq!(values.as_slice(), &[7]);
    assert!(values.try_reserve(usize::MAX).is_err());
    assert_eq!(values.as_slice(), &[7]);
}
