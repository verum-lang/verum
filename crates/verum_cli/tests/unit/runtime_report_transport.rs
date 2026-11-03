use super::*;
use verum_common::{CbgrViolationKind, runtime_diagnostics::*};

fn report() -> RuntimeReport {
    RuntimeReport {
        schema: RuntimeReportSchema::V2,
        export_loss: Some(RuntimeExportLoss::default()),
        run_id: "exact-run".into(),
        source_sha256: "a".repeat(64).into(),
        prepared_source_sha256: "b".repeat(64).into(),
        wrapper_lines: 1,
        entry_source_path: Some("/exact/source.vr".into()),
        compiler_version: "test".into(),
        observation: RuntimeObservation::Captured,
        cbgr: Some(CbgrReport {
            schema: CbgrSchema::DispatchV1,
            capacity: MAX_CBGR_EVENTS,
            checks_enabled_at_start: true,
            truncated: true,
            events: (0..MAX_CBGR_EVENTS)
                .map(|pc| CbgrEvent {
                    kind: CbgrViolationKind::GenerationMismatch,
                    target_kind: CbgrTargetKind::Register,
                    target: u32::MAX.to_string().into(),
                    expected: Some(u64::MAX.to_string().into()),
                    actual: Some("0".into()),
                    function_id: Some(1),
                    function: Some("\0".repeat(256).into()),
                    pc: pc as u32,
                    opcode: 20,
                    source: Some(CbgrSourceLocation {
                        path: "\0".repeat(2048).into(),
                        line: 1,
                        column: 1,
                    }),
                })
                .collect(),
        }),
        method_dispatch: Some(MethodDispatchReport {
            schema: MethodDispatchSchema::V1,
            capacity: MAX_METHOD_DISPATCH_SITES,
            dropped_calls: u64::MAX,
            rows: (0..MAX_METHOD_DISPATCH_SITES)
                .map(|id| MethodDispatchRow {
                    module_instance: id,
                    module: Some("\0".repeat(256).into()),
                    function_id: id as u32,
                    function: Some("\0".repeat(256).into()),
                    pc: 3,
                    selector_id: 7,
                    selector: Some("\0".repeat(256).into()),
                    outcome: MethodDispatchOutcome::Failed,
                    count: u64::MAX,
                    inclusive_ns: u64::MAX,
                    max_ns: u64::MAX,
                })
                .collect(),
        }),
        async_tasks: Some(AsyncTaskReport {
            schema: AsyncTaskSchema::V1,
            capacity: MAX_ASYNC_TASK_ROWS,
            omitted_tasks: u64::MAX,
            rows: (0..MAX_ASYNC_TASK_ROWS)
                .map(|id| AsyncTaskRow {
                    task_id: id as u64,
                    function_id: Some(1),
                    module: Some("\0".repeat(256).into()),
                    function: Some("\0".repeat(256).into()),
                    entered: true,
                    status: AsyncTaskStatus::Failed,
                    error: Some("\0".repeat(2048).into()),
                    error_truncated: false,
                })
                .collect(),
        }),
    }
}

#[test]
fn worst_case_json_escaping_obeys_one_budget_without_changing_retained_failures() {
    let original = report();
    assert!(json_size(&original).unwrap() > MAX_RUNTIME_REPORT_BYTES as usize * 4);
    let bytes = encode_bounded(original.clone()).unwrap();
    assert!(bytes.len() <= MAX_RUNTIME_REPORT_BYTES as usize);
    let bounded: RuntimeReport = serde_json::from_slice(&bytes).unwrap();
    bounded
        .validate_binding("exact-run", &"a".repeat(64))
        .unwrap();
    assert!(
        bounded
            .validate_binding("other-run", &"a".repeat(64))
            .is_err()
    );
    assert_eq!(
        bounded.prepared_source_sha256,
        original.prepared_source_sha256
    );
    assert_eq!(bounded.entry_source_path, original.entry_source_path);
    let loss = bounded.export_loss.unwrap();
    assert_eq!(
        bounded.cbgr.as_ref().unwrap().events.len() + loss.cbgr_events,
        MAX_CBGR_EVENTS
    );
    assert_eq!(
        bounded.method_dispatch.as_ref().unwrap().rows.len() + loss.method_sites,
        MAX_METHOD_DISPATCH_SITES
    );
    let tasks = bounded.async_tasks.as_ref().unwrap();
    assert_eq!(tasks.rows.len() + loss.task_rows, MAX_ASYNC_TASK_ROWS);
    assert!(loss.cbgr_events + loss.method_sites + loss.task_rows > 0);
    assert!(bounded.cbgr.as_ref().unwrap().truncated);
    assert_eq!(
        bounded.method_dispatch.as_ref().unwrap().dropped_calls,
        u64::MAX
    );
    assert_eq!(tasks.omitted_tasks, u64::MAX);
    for row in &tasks.rows {
        assert_eq!(row.status, AsyncTaskStatus::Failed);
        assert_eq!(
            row.error,
            original.async_tasks.as_ref().unwrap().rows[row.task_id as usize].error
        );
        assert!(!row.error_truncated);
    }
    if let Some(last) = tasks.rows.last() {
        assert_eq!(last.task_id, (MAX_ASYNC_TASK_ROWS - 1) as u64);
    }
    assert_eq!(
        encode_bounded(bounded).unwrap(),
        bytes,
        "re-export must preserve loss accounting"
    );
}

#[test]
fn small_limits_keep_identity_and_exact_omissions_even_if_every_observation_is_removed() {
    let original = report();
    let bytes = encode_with_limit(original.clone(), 1600).unwrap();
    assert!(bytes.len() <= 1600);
    let bounded: RuntimeReport = serde_json::from_slice(&bytes).unwrap();
    bounded
        .validate_binding("exact-run", &"a".repeat(64))
        .unwrap();
    let loss = bounded.export_loss.unwrap();
    assert_eq!(loss.cbgr_events, MAX_CBGR_EVENTS);
    assert_eq!(loss.method_sites, MAX_METHOD_DISPATCH_SITES);
    assert_eq!(loss.task_rows, MAX_ASYNC_TASK_ROWS);
    assert_eq!(bounded.observation, RuntimeObservation::Captured);
    assert!(encode_with_limit(original, 1).is_err());
}

#[test]
fn ordinary_and_unavailable_reports_are_not_relabelled_or_truncated() {
    let mut original = report();
    original.cbgr.as_mut().unwrap().events.truncate(1);
    original.cbgr.as_mut().unwrap().events[0].source = Some(CbgrSourceLocation {
        path: "🧮/é/\"quoted\"\\source.vr".into(),
        line: 2,
        column: 1,
    });
    original.method_dispatch = None;
    original.async_tasks = None;
    let bytes = encode_bounded(original.clone()).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&original).unwrap());
    original.cbgr = None;
    original.observation = RuntimeObservation::Unavailable;
    let bytes = encode_bounded(original.clone()).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&original).unwrap());
    original.export_loss.as_mut().unwrap().task_rows = 1;
    assert!(encode_bounded(original).is_err());
}

#[test]
fn invalid_capture_is_refused_before_any_oversized_rows_can_be_removed() {
    let mut invalid = report();
    invalid.async_tasks.as_mut().unwrap().rows[0].status = AsyncTaskStatus::Completed;
    assert!(encode_bounded(invalid).is_err());
    let mut invalid = report();
    invalid.schema = RuntimeReportSchema::V1;
    assert!(encode_bounded(invalid).is_err());
    for update in [
        |loss: &mut RuntimeExportLoss| loss.cbgr_events = usize::MAX,
        |loss: &mut RuntimeExportLoss| loss.method_sites = usize::MAX,
        |loss: &mut RuntimeExportLoss| loss.task_rows = usize::MAX,
    ] {
        let mut invalid = report();
        update(invalid.export_loss.as_mut().unwrap());
        assert!(
            encode_bounded(invalid).is_err(),
            "invalid counts must not panic or wrap"
        );
    }
}

#[test]
fn partial_task_export_retains_the_recent_suffix_and_preserves_each_error() {
    let mut original = report();
    original.cbgr.as_mut().unwrap().events.clear();
    original.cbgr.as_mut().unwrap().truncated = false;
    original.method_dispatch = None;
    let first_size = json_size(&original.async_tasks.as_ref().unwrap().rows[0]).unwrap();
    let limit = json_size(&original).unwrap() - first_size / 2;
    let bytes = encode_with_limit(original.clone(), limit).unwrap();
    assert!(bytes.len() <= limit);
    let bounded: RuntimeReport = serde_json::from_slice(&bytes).unwrap();
    let omitted = bounded.export_loss.unwrap().task_rows;
    assert!(omitted > 0 && omitted < MAX_ASYNC_TASK_ROWS);
    let expected = &original.async_tasks.as_ref().unwrap().rows.as_slice()[omitted..];
    let actual = &bounded.async_tasks.as_ref().unwrap().rows;
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(actual[0].task_id, omitted as u64);
    assert_eq!(
        actual.last().unwrap().task_id,
        (MAX_ASYNC_TASK_ROWS - 1) as u64
    );
}
