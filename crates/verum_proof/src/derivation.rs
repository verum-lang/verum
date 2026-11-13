//! Drive dependent typing premises on an explicit stack. Each rule suspends at
//! its next premise. The driver polls only the top frame, so a suspended parent
//! never occupies the native call stack. Both kernels supply their own rules.
use crate::proof_checker::{CheckError, Term};
use std::{
    borrow::Cow,
    cell::Cell,
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

pub(super) type Input<'a> = Cow<'a, Term>;
pub(super) type Children<'a, 'source> = fn(&'source Term) -> Input<'a>;

/// Children of the original certificate borrow its syntax. A premise produced
/// by substitution owns any subterm that outlives the suspended rule.
pub(super) fn copy_child<'a>(term: &Term) -> Input<'a> {
    Cow::Owned(term.clone())
}

const DERIVATION_PREMISE_LIMIT: usize = 1 << 20;

type Frame<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;

#[derive(Clone, Default)]
pub(super) struct WorkStack<'a> {
    next: Rc<Cell<Option<Frame<'a>>>>,
}

impl<'a> WorkStack<'a> {
    pub(super) fn premise<T: 'a, F: Future<Output = T> + 'a>(
        &self,
        rule: F,
    ) -> impl Future<Output = T> + 'a + use<'a, T, F> {
        let next = Rc::clone(&self.next);
        let answer = Rc::new(Cell::new(None));
        let returned = Rc::clone(&answer);
        let mut frame: Option<Frame<'a>> = Some(Box::pin(async move {
            returned.set(Some(rule.await));
        }));
        poll_fn(move |_| match frame.take() {
            Some(frame) => {
                next.set(Some(frame));
                Poll::Pending
            }
            None => Poll::Ready(
                answer
                    .take()
                    .expect("a completed premise returned its judgment"),
            ),
        })
    }

    pub(super) fn run<T>(
        &self,
        root: impl Future<Output = Result<T, CheckError>>,
    ) -> Result<T, CheckError> {
        // A derivation may introduce premises beyond the input's syntax (e.g.
        // unfolding a local definition). Bound those independently of WHNF.
        self.run_with_budget(root, DERIVATION_PREMISE_LIMIT)
    }

    pub(super) fn run_with_budget<T>(
        &self,
        root: impl Future<Output = Result<T, CheckError>>,
        mut remaining: usize,
    ) -> Result<T, CheckError> {
        let mut root = Box::pin(root);
        let mut pending: Vec<Frame<'a>> = Vec::new();
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Some(frame) = pending.last_mut() {
                if frame.as_mut().poll(&mut context).is_ready() {
                    pending.pop();
                    continue;
                }
            } else if let Poll::Ready(result) = root.as_mut().poll(&mut context) {
                return result;
            }
            let premise = self
                .next
                .take()
                .expect("rules suspend only for a requested premise");
            remaining = remaining
                .checked_sub(1)
                .ok_or(CheckError::DerivationLimit)?;
            pending.push(premise);
        }
    }
}
