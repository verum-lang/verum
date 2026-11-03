use serde_json::{Value, json};
use verum_workbench::{execution::ExecutionPlan, runtime_diagnostics::read_report};

fn report() -> Value {
    json!({
        "schema":"verum.runtime-report/1", "run_id":"current", "source_sha256":"a".repeat(64),
        "prepared_source_sha256":"b".repeat(64), "wrapper_lines":1, "entry_source_path":null,
        "compiler_version":"0.1.0", "observation":"captured",
        "cbgr":{"schema":"verum.cbgr.dispatch/1", "capacity":256,"checks_enabled_at_start":true,"truncated":false,"events":[{
            "kind":"GenerationMismatch","target_kind":"register","target":"1",
            "expected":"18446744073709551615","actual":"0","function_id":0,"function":"main",
            "pc":4,"opcode":20,"source":null
        }]}
    })
}

#[test]
fn reports_are_bound_to_invocation_and_source_with_lossless_counters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime.json");
    std::fs::write(&path, serde_json::to_vec(&report()).unwrap()).unwrap();
    let observed = read_report(&path, "current", &"a".repeat(64)).unwrap();
    assert_eq!(
        observed.cbgr.unwrap().events[0].expected.as_deref(),
        Some("18446744073709551615")
    );
    assert!(read_report(&path, "previous", &"a".repeat(64)).is_err());
    assert!(read_report(&path, "current", &"c".repeat(64)).is_err());
}

#[test]
fn async_failures_survive_transport_and_cannot_be_relabelled_as_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime.json");
    let mut body = report();
    body["async_tasks"] = json!({
        "schema":"verum.async-tasks/1", "capacity":1, "omitted_tasks":"9007199254740993",
        "rows":[{"task_id":"9007199254740994", "function_id":7, "module":"app", "function":"handler",
            "entered":true, "status":"failed", "error":"original failure", "error_truncated":false}]
    });
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    let observed = read_report(&path, "current", &"a".repeat(64)).unwrap();
    let tasks = observed.async_tasks.unwrap();
    assert_eq!(tasks.rows[0].task_id, 9007199254740994);
    assert_eq!(serde_json::to_value(&tasks).unwrap()["rows"][0]["task_id"], "9007199254740994");
    assert_eq!(tasks.rows[0].error.as_deref(), Some("original failure"));
    assert!(read_report(&path, "other", &"a".repeat(64)).is_err());
    body["async_tasks"]["rows"][0]["status"] = json!("completed");
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    assert!(read_report(&path, "current", &"a".repeat(64)).is_err());
}

#[test]
fn absent_malformed_unsupported_and_inconsistent_evidence_never_becomes_empty_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime.json");
    assert!(read_report(&path, "current", &"a".repeat(64)).is_err());
    for (pointer, value) in [
        ("/schema", json!("verum.runtime-report/999")),
        ("/cbgr/schema", json!("other")),
        ("/observation", json!("unavailable")),
        ("/cbgr", Value::Null),
        ("/cbgr/capacity", json!(257)),
        ("/cbgr/capacity", json!(0)),
        ("/cbgr/events/0/expected", json!(18446744073709551615_u64)),
        ("/cbgr/events/0/actual", json!("18446744073709551616")),
        (
            "/cbgr/events/0/source",
            json!({"path":"x.vr","line":0,"column":1}),
        ),
    ] {
        let mut body = report();
        *body.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(
            read_report(&path, "current", &"a".repeat(64)).is_err(),
            "{pointer}"
        );
    }
    std::fs::write(&path, b"{\"schema\":").unwrap();
    assert!(read_report(&path, "current", &"a".repeat(64)).is_err());
    std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(read_report(&path, "current", &"a".repeat(64)).is_err());
    #[cfg(unix)]
    {
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(read_report(&link, "current", &"a".repeat(64)).is_err());
    }
}

#[test]
fn versioned_export_losses_are_distinct_from_capture_losses_and_require_valid_accounting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runtime.json");
    let mut body = report();
    body["schema"] = json!("verum.runtime-report/2");
    body["export_loss"] = json!({"cbgr_events":255,"method_sites":0,"task_rows":0});
    std::fs::write(&path, serde_json::to_vec(&body).unwrap()).unwrap();
    let observed = read_report(&path, "current", &"a".repeat(64)).unwrap();
    assert_eq!(observed.export_loss.unwrap().cbgr_events, 255);
    assert_eq!(observed.cbgr.unwrap().events.len(), 1);
    for (pointer, value) in [
        ("/schema", json!("verum.runtime-report/1")),
        ("/export_loss", Value::Null),
        ("/export_loss/cbgr_events", json!(256)),
        ("/export_loss/task_rows", json!(1)),
    ] {
        let mut bad = body.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
        assert!(read_report(&path, "current", &"a".repeat(64)).is_err(), "{pointer}");
    }
}

#[test]
fn diagnostics_are_explicit_and_apply_only_to_runs() {
    let parse = |params: Value| ExecutionPlan::parse(&params, "verum".as_ref(), "proof".as_ref());
    let mut request = json!({"kind":"run","source":"source","path":"entry.vr"});
    assert_eq!(
        parse(request.clone()).unwrap().runtime_diagnostics,
        Some(false)
    );
    request["runtime_diagnostics"] = json!(true);
    assert_eq!(
        parse(request.clone()).unwrap().runtime_diagnostics,
        Some(true)
    );
    for invalid in [json!("true"), Value::Null, json!(1)] {
        request["runtime_diagnostics"] = invalid;
        assert!(parse(request.clone()).is_err());
    }
    request["runtime_diagnostics"] = json!(false);
    request["kind"] = json!("check");
    assert!(parse(request.clone()).is_err());
    request["kind"] = json!("proof");
    request["declaration"] = json!("identity");
    assert!(parse(request).is_err());
}
