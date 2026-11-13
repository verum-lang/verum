use super::{CheckError, Context, Term, WorkStack, request_infer};
use crate::admission::{ReplayOutcome, classify_check_failure};
use std::{borrow::Cow, rc::Rc};

#[test]
fn a_rule_returns_a_judgment_only_after_all_requested_premises_finish() {
    let statement = Term::pi(Term::universe(0), Term::var(0));
    for (budget, expected) in [
        (2, Err(CheckError::DerivationLimit)),
        (3, Ok(Term::universe(1))),
    ] {
        let work = WorkStack::default();
        let root = request_infer(&work, Rc::new(Context::new()), Cow::Borrowed(&statement));
        assert_eq!(work.run_with_budget(root, budget), expected);
    }
    assert_eq!(
        classify_check_failure(&CheckError::DerivationLimit),
        (ReplayOutcome::ResourceExhausted, "derivation_limit"),
    );
}
