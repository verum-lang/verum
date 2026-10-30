#![cfg(unix)]
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use verum_common::List;
use verum_workbench::{jobs::Jobs, workspace::digest};

fn runner(root: &Path, body: &str) -> Jobs {
    let executable = root.join("fixture");
    std::fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    Jobs {
        jobs: List::new(),
        verum: executable.clone(),
        proof: executable,
        root: root.into(),
    }
}
async fn done(jobs: &Jobs) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipts = jobs.list().await;
            if receipts[0]["status"] != "running" {
                return receipts[0].clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("runner must settle")
}
fn start(jobs: &mut Jobs, source: &str, timeout: u64) -> Value {
    jobs.start(
        &json!({"kind":"run","path":"lesson.vr","source":source,"timeout_ms":timeout}),
        jobs.root.join("lesson.vr"),
    )
    .unwrap()
}
#[tokio::test]
async fn drains_both_pipes_and_runs_an_immutable_entry_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let mut jobs = runner(
        dir.path(),
        "sleep 0.1\ncat\ndd if=/dev/zero bs=4096 count=20 2>/dev/null\ndd if=/dev/zero bs=4096 count=20 >&2 2>/dev/null",
    );
    std::fs::write(dir.path().join("lesson.vr"), "disk").unwrap();
    start(&mut jobs, "exact submitted source", 3000);
    std::fs::write(dir.path().join("lesson.vr"), "changed during execution").unwrap();
    let receipt = done(&jobs).await;
    assert_eq!(receipt["status"], "succeeded");
    assert!(
        receipt["stdout"]
            .as_str()
            .unwrap()
            .starts_with("exact submitted source")
    );
    assert!(receipt["stderr"].as_str().unwrap().len() >= 81920);
    assert_eq!(
        receipt["source_hash"],
        json!(digest(b"exact submitted source"))
    );
    let saved = Path::new(receipt["evidence_path"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(saved.join("source.vr")).unwrap(),
        "exact submitted source"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(saved.join("receipt.json")).unwrap())
            .unwrap(),
        receipt
    );
}
#[tokio::test]
async fn output_budget_and_deadline_are_distinct_from_success() {
    let dir = tempfile::tempdir().unwrap();
    let mut jobs = runner(dir.path(), "dd if=/dev/zero bs=4096 count=1000 2>/dev/null");
    start(&mut jobs, "bounded", 3000);
    let receipt = done(&jobs).await;
    assert_eq!(receipt["status"], "exhausted");
    assert!(receipt["stdout"].as_str().unwrap().len() <= 262144);
    let mut jobs = runner(dir.path(), "sleep 30");
    start(&mut jobs, "deadline", 60);
    assert_eq!(done(&jobs).await["status"], "exhausted");
}
#[tokio::test]
async fn cancellation_settles_even_when_a_descendant_holds_output_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut jobs = runner(dir.path(), "sleep 30 &\nwait");
    let run = start(&mut jobs, "cancel", 30000);
    jobs.cancel(run["id"].as_str().unwrap()).await.unwrap();
    assert_eq!(done(&jobs).await["status"], "cancelled");
}
#[tokio::test]
async fn zero_exit_is_not_sufficient_for_a_proof() {
    let dir = tempfile::tempdir().unwrap();
    let source = "theorem identity(A: Type, x: A): A proof = x;";
    let response = json!({"status":"rejected","source":{"sha256":digest(source.as_bytes())}});
    let mut jobs = runner(
        dir.path(),
        &format!("cat >/dev/null\nprintf '%s' '{}'", response),
    );
    jobs.start(
        &json!({"kind":"proof","path":"proof.vr","source":source,"declaration":"identity"}),
        dir.path().join("proof.vr"),
    )
    .unwrap();
    assert_eq!(done(&jobs).await["status"], "failed");
    let mut jobs = runner(
        dir.path(),
        "cat >/dev/null\nprintf '%s' '{\"status\":\"verified\",\"source\":{\"sha256\":\"another-source\"}}'",
    );
    jobs.start(
        &json!({"kind":"proof","path":"proof.vr","source":source,"declaration":"identity"}),
        dir.path().join("proof.vr"),
    )
    .unwrap();
    assert_eq!(done(&jobs).await["status"], "inconclusive");
}

#[tokio::test]
async fn crashed_checker_is_inconclusive_and_cpu_exhaustion_is_not_rejection() {
    for (body, expected) in [
        ("cat >/dev/null\nexit 7", "inconclusive"),
        ("cat >/dev/null\nkill -XCPU $$", "exhausted"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut jobs = runner(dir.path(), body);
        jobs.start(&json!({"kind":"proof","path":"proof.vr","source":"theorem identity(A:Type,x:A):A proof=x;","declaration":"identity"}),dir.path().join("proof.vr")).unwrap();
        assert_eq!(done(&jobs).await["status"], expected);
    }
}

#[tokio::test]
async fn retained_receipts_restore_and_unchanged_output_is_not_retransmitted() {
    let dir = tempfile::tempdir().unwrap();
    let mut jobs = runner(dir.path(), "cat");
    start(&mut jobs, "exact snapshot", 3000);
    let receipt = done(&jobs).await;
    let known = json!({receipt["id"].as_str().unwrap():receipt["version"]});
    assert_eq!(jobs.changes(&known).await, json!([]));
    let mut restored = runner(dir.path(), "cat");
    restored.restore().unwrap();
    assert_eq!(restored.list().await, json!([receipt]));
    let path = Path::new(receipt["evidence_path"].as_str().unwrap()).join("source.vr");
    std::fs::write(path, "tampered input").unwrap();
    let mut restored = runner(dir.path(), "cat");
    restored.restore().unwrap();
    assert_eq!(
        restored.list().await,
        json!([]),
        "Unbound retained input must not become current evidence"
    );
}
