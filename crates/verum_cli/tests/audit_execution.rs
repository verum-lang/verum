#[path = "../src/commands/audit_execution.rs"]
mod audit_execution;

#[test]
fn old_success_files_cannot_become_current_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.json");
    std::fs::write(&report, br#"{"load_bearing":true,"run_id":"old"}"#).unwrap();
    for result in [Ok(()), Err("gate did not finish")] {
        let receipt = audit_execution::capture("current", "test", || result);
        assert_eq!(receipt["run_id"], "current");
        assert_eq!(receipt["load_bearing"], false);
        assert_eq!(receipt["proof_status"], "no_bound_proof_evidence");
        assert_eq!(
            receipt["execution_status"],
            if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
        );
    }
}

#[test]
fn publication_replaces_complete_json_and_propagates_write_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.json");
    std::fs::write(&path, b"old success").unwrap();
    let value = serde_json::json!({"run_id":"fresh", "execution_status":"running"});
    audit_execution::publish(&path, &value).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
        value
    );
    assert!(audit_execution::publish(dir.path(), &value).is_err());
    assert!(audit_execution::publish(&dir.path().join("missing/report.json"), &value).is_err());
}
