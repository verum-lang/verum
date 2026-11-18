use super::*;

#[test]
fn interleaved_reviews_keep_their_own_decision_dispatch_and_outcome() {
    let mut window = Window::default();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    assert_eq!(window.check(0, None).status, Status::Empty);
    assert_eq!(window.check(1, None).status, Status::Incomplete);
    window.push("turn", a, "verum_check", Step::Approved);
    window.push("turn", b, "verum_check", Step::Approved);
    window.push("turn", b, "verum_check", Step::Dispatched);
    window.push("turn", a, "verum_check", Step::Dispatched);
    window.push("turn", b, "verum_check", Step::Failed);
    assert_eq!(window.check(0, None).unfinished, 1);
    window.push("turn", a, "verum_check", Step::Succeeded);
    window.push("turn", Uuid::new_v4(), "verum_propose", Step::Declined);
    let refused = Uuid::new_v4();
    window.push("turn", refused, "verum_read", Step::Approved);
    window.push("turn", refused, "verum_read", Step::RefusedBeforeDispatch);
    let check = window.check(0, None);
    assert_eq!(check.status, Status::Checked);
    assert_eq!(
        (
            check.succeeded,
            check.failed,
            check.declined,
            check.refused_before_dispatch
        ),
        (1, 1, 1, 1)
    );
}

#[test]
fn counterexample_identifies_the_first_broken_edge_on_its_actual_occurrence() {
    use Step::*;
    for sequence in [
        &[Dispatched][..],
        &[Failed],
        &[Approved, Succeeded],
        &[Declined, Dispatched],
        &[Approved, Dispatched, Dispatched],
        &[Approved, Dispatched, Failed, Succeeded],
        &[Approved, Dispatched, MisboundReceipt],
    ] {
        let id = Uuid::new_v4();
        let mut window = Window::default();
        for step in sequence {
            window.push("turn", id, "verum_run", *step);
        }
        let check = window.check(0, None);
        assert_eq!(check.status, Status::Violated, "{sequence:?}");
        let violation = check.violation.unwrap();
        assert_eq!(violation.observed.ordinal, sequence.len() as u64);
        assert_eq!(violation.observed.review_id, id);
    }
    for (turn, tool, fresh_id) in [
        ("other", "verum_read", false),
        ("turn", "verum_run", false),
        ("turn", "verum_read", true),
    ] {
        let review = Uuid::new_v4();
        let mut window = Window::default();
        window.push("turn", review, "verum_read", Approved);
        window.push(
            turn,
            if fresh_id { Uuid::new_v4() } else { review },
            tool,
            Dispatched,
        );
        assert_eq!(window.check(0, None).status, Status::Violated);
    }
}

#[test]
fn eviction_distinguishes_a_missing_prefix_from_an_observed_contradiction() {
    let journal = Journal::default();
    let review = Uuid::new_v4();
    journal.record("author", "turn", review, "verum_run", Step::Approved);
    journal.record("author", "turn", review, "verum_run", Step::Dispatched);
    journal.record("author", "turn", review, "verum_run", Step::Succeeded);
    for _ in 0..RETAINED_STEPS - 2 {
        journal.record(
            "author",
            "turn",
            Uuid::new_v4(),
            "verum_read",
            Step::Declined,
        );
    }
    let lost = journal.check("author", 0, None);
    assert_eq!(
        (lost.status, lost.retained, lost.omitted),
        (Status::Incomplete, RETAINED_STEPS, 1)
    );
    assert!(lost.violation.is_none());
    journal.record("reviewer", "turn", review, "verum_read", Step::Declined);
    assert_eq!(journal.check("reviewer", 0, None).status, Status::Checked);
    journal.record("author", "turn", review, "verum_run", Step::Dispatched);
    let broken = journal.check("author", 0, None);
    assert_eq!(broken.status, Status::Violated);
    let edge = broken.violation.unwrap();
    assert_eq!(edge.previous.unwrap().step, Step::Succeeded);
    assert_eq!(edge.observed.step, Step::Dispatched);
}

#[test]
fn missing_receipt_origin_and_the_active_inspection_have_distinct_coverage() {
    let mut window = Window::default();
    let inspection = Uuid::new_v4();
    window.push("A", inspection, "verum_action_sequence", Step::Approved);
    window.push("A", inspection, "verum_action_sequence", Step::Dispatched);
    assert_eq!(window.check(0, None).status, Status::Incomplete);
    let snapshot = window.check(0, Some(inspection));
    assert_eq!(snapshot.status, Status::Empty);
    assert_eq!(snapshot.inspecting_review, Some(inspection));
    let job = Uuid::new_v4();
    for step in [Step::Approved, Step::Dispatched, Step::ReceiptOriginMissing] {
        window.push("A", job, "verum_run", step);
    }
    let result = window.check(0, Some(inspection));
    assert_eq!(
        (result.status, result.unbound_receipts, result.retained),
        (Status::Incomplete, 1, 3)
    );
    assert!(result.violation.is_none());
}

#[test]
fn execution_outcomes_require_the_recorded_owner_and_exact_review_id() {
    use crate::agent_runtime::{ReviewOrigin, TurnIdentity};
    use serde_json::json;
    let origin = ReviewOrigin {
        owner: TurnIdentity {
            connection: "fixture".into(),
            conversation: "author".into(),
            turn: "A".into(),
        },
        review_id: Some(Uuid::new_v4()),
    };
    let agent = serde_json::to_value(&origin).unwrap();
    assert_eq!(receipt_outcome(&origin, &agent, true), Step::Succeeded);
    assert_eq!(receipt_outcome(&origin, &agent, false), Step::Failed);
    for field in ["connection", "conversation", "turn", "review_id"] {
        let mut changed = agent.clone();
        changed[field] = json!("unrelated");
        assert_eq!(
            receipt_outcome(&origin, &changed, true),
            Step::MisboundReceipt
        );
        changed.as_object_mut().unwrap().remove(field);
        assert_eq!(
            receipt_outcome(&origin, &changed, false),
            Step::ReceiptOriginMissing
        );
    }
    assert_eq!(
        receipt_outcome(&origin, &Value::Null, true),
        Step::ReceiptOriginMissing
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_evicted_execution_receipt_reports_incomplete_ordering() {
    use crate::{Config, Studio, agent_runtime::Turns};
    use serde_json::json;
    use std::time::Duration;

    let root = tempfile::tempdir().unwrap();
    let studio = Studio::open(Config {
        workspace: root.path().into(),
        codex: "unused".into(),
        verum: "/usr/bin/true".into(),
        proof: "unused".into(),
        port: 0,
    })
    .unwrap();
    let turns = Turns::new(Uuid::new_v4());
    let owner = turns
        .get("author", "A")
        .unwrap()
        .child(Uuid::new_v4())
        .unwrap();
    owner.record_action("verum_check", Step::Approved);
    let request = json!({"tool":"verum_check","arguments":{"path":"main.vr","source":""}});
    let execute = studio.tool_owned(&request, Some(&owner));
    let evict = async {
        // The first receipt read sees the running process. Hold the jobs index until the
        // worker settles, then evict its terminal entry before the next tool poll.
        let mut jobs = loop {
            let jobs = studio.jobs.lock().await;
            if !jobs.jobs.is_empty() {
                break jobs;
            }
            drop(jobs);
            tokio::task::yield_now().await;
        };
        let receipt = jobs.jobs[0].receipt.clone();
        while receipt.lock().await.status == "running" {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(receipt.lock().await.status.as_str(), "succeeded");
        jobs.jobs.remove(0);
    };
    let (reply, ()) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(biased; execute, evict)
    })
    .await
    .unwrap();
    assert!(reply.unwrap_err().is::<crate::ReceiptUnavailable>());
    let check = turns.journal.check("author", 0, None);
    assert_eq!(
        (check.status, check.unbound_receipts, check.failed),
        (Status::Incomplete, 1, 0)
    );
}
