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
        method_dispatch: None,
        async_tasks: None,
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
fn task_snapshots_refuse_impossible_outcomes_and_oversized_payloads() {
    use verum_common::runtime_diagnostics::*;
    let mut report = report();
    report.async_tasks = Some(AsyncTaskReport {
        schema: AsyncTaskSchema::V1, capacity: 1, omitted_tasks: u64::MAX,
        rows: [AsyncTaskRow {
            task_id: u64::MAX, function_id: Some(1), module: Some("app".into()),
            function: Some("worker".into()), entered: true, status: AsyncTaskStatus::Failed,
            error: Some("original failure".into()), error_truncated: false,
        }].into_iter().collect(),
    });
    let valid = |r: &RuntimeReport| r.validate_binding("current-run", &"a".repeat(64)).is_ok();
    assert!(valid(&report));
    assert!(!report.validate_binding("previous-run", &"a".repeat(64)).is_ok());
    let mutations: [fn(&mut AsyncTaskRow); 5] = [
        |row| row.status = AsyncTaskStatus::Completed,
        |row| row.error = None,
        |row| row.entered = false,
        |row| row.function_id = None,
        |row| row.error = Some("x".repeat(MAX_TASK_ERROR_BYTES + 1).into()),
    ];
    for mutate in mutations {
        let mut bad = report.clone();
        mutate(&mut bad.async_tasks.as_mut().unwrap().rows[0]);
        assert!(!valid(&bad));
    }
    let mut bad = report.clone();
    bad.async_tasks.as_mut().unwrap().rows.clear();
    assert!(!valid(&bad));
    let mut bad = report.clone();
    bad.async_tasks.as_mut().unwrap().capacity = MAX_ASYNC_TASK_ROWS + 1;
    assert!(!valid(&bad));
    report.cbgr = None;
    report.observation = RuntimeObservation::Unavailable;
    assert!(!valid(&report));
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

#[test]
fn method_measurements_require_this_execution_and_consistent_counts() {
    use verum_common::runtime_diagnostics::{
        MethodDispatchOutcome, MethodDispatchReport, MethodDispatchRow, MethodDispatchSchema,
    };
    let mut report = report();
    let source = report.source_sha256.clone();
    report.method_dispatch = Some(MethodDispatchReport {
        schema: MethodDispatchSchema::V1,
        capacity: 1,
        dropped_calls: 0,
        rows: [MethodDispatchRow {
            module_instance: 0,
            module: None,
            function_id: 8,
            function: None,
            selector_id: 4,
            selector: None,
            pc: 3,
            outcome: MethodDispatchOutcome::CompletedInline,
            count: 3,
            inclusive_ns: 15,
            max_ns: 6,
        }]
        .into_iter()
        .collect(),
    });
    assert!(report.validate_binding("current-run", &source).is_ok());
    report.method_dispatch.as_mut().unwrap().rows[0].max_ns = 4;
    assert!(report.validate_binding("current-run", &source).is_err());
    report.method_dispatch.as_mut().unwrap().rows.clear();
    report.observation = RuntimeObservation::Unavailable;
    report.cbgr = None;
    assert!(report.validate_binding("current-run", &source).is_err());
}
