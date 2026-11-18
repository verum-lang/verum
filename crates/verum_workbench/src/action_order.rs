//! Finite-trace checking of Studio's own reviewed tool calls, scoped to one provider connection.
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use verum_common::{List, Map, Text};

const RETAINED_STEPS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    Approved,
    Declined,
    RefusedBeforeDispatch,
    Dispatched,
    Succeeded,
    Failed,
    ReceiptUnavailable,
    ReceiptOriginMissing,
    MisboundReceipt,
}

pub(crate) fn receipt_outcome(
    expected: &crate::agent_runtime::ReviewOrigin,
    agent: &Value,
    succeeded: bool,
) -> Step {
    let Some(review) = expected.review_id else {
        return Step::ReceiptOriginMissing;
    };
    let review = review.to_string();
    let mut missing = false;
    for (field, expected) in [
        ("connection", expected.owner.connection.as_str()),
        ("conversation", expected.owner.conversation.as_str()),
        ("turn", expected.owner.turn.as_str()),
        ("review_id", review.as_str()),
    ] {
        match agent[field].as_str() {
            Some(value) if value != expected => return Step::MisboundReceipt,
            None => missing = true,
            _ => {}
        }
    }
    if missing {
        Step::ReceiptOriginMissing
    } else if succeeded {
        Step::Succeeded
    } else {
        Step::Failed
    }
}

#[derive(Clone, Serialize)]
struct Observation {
    ordinal: u64,
    review_id: Uuid,
    turn: Text,
    tool: Text,
    step: Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Empty,
    Checked,
    Incomplete,
    Violated,
}

#[derive(Serialize)]
struct Violation {
    previous: Option<Observation>,
    observed: Observation,
}

#[derive(Serialize)]
pub(crate) struct Check {
    rule: &'static str,
    status: Status,
    retained: usize,
    omitted: u64,
    awaiting_decision: usize,
    unfinished: usize,
    succeeded: usize,
    failed: usize,
    declined: usize,
    refused_before_dispatch: usize,
    unbound_receipts: usize,
    violation: Option<Violation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inspecting_review: Option<Uuid>,
}

#[derive(Default)]
struct Window {
    observations: List<Observation>,
    omitted: u64,
}

impl Window {
    fn push(&mut self, turn: &str, review_id: Uuid, tool: &str, step: Step) {
        let event = Observation {
            ordinal: self.omitted + self.observations.len() as u64 + 1,
            turn: turn.into(),
            review_id,
            tool: tool.into(),
            step,
        };
        if self.observations.len() < RETAINED_STEPS {
            self.observations.push(event);
        } else {
            let oldest = (self.omitted % RETAINED_STEPS as u64) as usize;
            self.observations[oldest] = event;
            self.omitted += 1;
        }
    }

    fn check(&self, awaiting_decision: usize, inspecting_review: Option<Uuid>) -> Check {
        let mut last = Map::<Uuid, &Observation>::new();
        let mut check = Check {
            rule: "studio.reviewed-tool-order/1",
            status: Status::Empty,
            retained: 0,
            omitted: self.omitted,
            awaiting_decision,
            inspecting_review,
            unfinished: 0,
            succeeded: 0,
            failed: 0,
            declined: 0,
            refused_before_dispatch: 0,
            unbound_receipts: 0,
            violation: None,
        };
        let oldest = (self.omitted % RETAINED_STEPS as u64) as usize;
        let (newer, older) = self.observations.split_at(oldest);
        for event in older.iter().chain(newer) {
            // A reviewed inspection reads its peers. Its own outcome is observable after it returns.
            if Some(event.review_id) == inspecting_review {
                continue;
            }
            check.retained += 1;
            let previous = last.insert(event.review_id, event);
            let valid = event.step != Step::MisboundReceipt
                && match previous {
                    Some(prior) => {
                        prior.turn == event.turn
                            && prior.tool == event.tool
                            && matches!(
                                (prior.step, event.step),
                                (
                                    Step::Approved,
                                    Step::Dispatched | Step::RefusedBeforeDispatch
                                ) | (
                                    Step::Dispatched,
                                    Step::Succeeded
                                        | Step::Failed
                                        | Step::ReceiptOriginMissing
                                        | Step::ReceiptUnavailable
                                )
                            )
                    }
                    // The first retained observation may have had a predecessor in the lost prefix.
                    None => {
                        self.omitted != 0 || matches!(event.step, Step::Approved | Step::Declined)
                    }
                };
            if !valid && check.violation.is_none() {
                check.violation = Some(Violation {
                    previous: previous.cloned(),
                    observed: event.clone(),
                });
            }
            match event.step {
                Step::Succeeded => check.succeeded += 1,
                Step::Failed => check.failed += 1,
                Step::Declined => check.declined += 1,
                Step::RefusedBeforeDispatch => check.refused_before_dispatch += 1,
                Step::ReceiptOriginMissing | Step::ReceiptUnavailable => {
                    check.unbound_receipts += 1
                }
                _ => {}
            }
        }
        check.unfinished = last
            .values()
            .filter(|event| matches!(event.step, Step::Approved | Step::Dispatched))
            .count();
        check.status = if check.violation.is_some() {
            Status::Violated
        } else if self.omitted != 0
            || awaiting_decision != 0
            || check.unfinished != 0
            || check.unbound_receipts != 0
        {
            Status::Incomplete
        } else if check.retained == 0 {
            Status::Empty
        } else {
            Status::Checked
        };
        check
    }
}

/// Each registered conversation has its own bounded observation window. Provider messages never enter it.
#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Map<Text, Window>>>);

impl Journal {
    pub(crate) fn record(
        &self,
        conversation: &str,
        turn: &str,
        review_id: Uuid,
        tool: &str,
        step: Step,
    ) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(conversation.into())
            .or_default()
            .push(turn, review_id, tool, step);
    }

    pub(crate) fn check(
        &self,
        conversation: &str,
        awaiting_decision: usize,
        inspecting_review: Option<Uuid>,
    ) -> Check {
        let windows = self.0.lock().unwrap_or_else(|error| error.into_inner());
        windows
            .get(&Text::from(conversation))
            .unwrap_or(&Window::default())
            .check(awaiting_decision, inspecting_review)
    }
}

#[cfg(test)]
#[path = "../tests/action_order/checks.rs"]
mod tests;
