//! Source-bound obligations and their conjunction in the existing calculus.
use super::{boolean, choose, truth};
use crate::{SourceBinding, SourceSpan};
use serde::{Deserialize, Serialize};
use verum_common::span::Span;
use verum_proof::proof_checker::{Certificate, Term};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    PreconditionCall,
    BodyCall,
    PostconditionCall,
    FunctionContract,
}

/// In-order component of the balanced Sigma proof bundle. Metadata locates the
/// source obligation; only the complete bundle's checked type supplies evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObligationBinding {
    pub owner: SourceBinding,
    pub kind: ObligationKind,
    pub call: Option<SourceSpan>,
    pub callee: Option<SourceBinding>,
}

pub(crate) struct Candidate {
    pub binding: ObligationBinding,
    pub certificate: Certificate,
}

#[derive(Clone)]
pub(super) struct CallRequirement {
    pub callee: String,
    pub span: Span,
    pub predicate: Term,
}
impl CallRequirement {
    pub fn guard(&mut self, condition: Term) {
        self.predicate = choose(condition, boolean(), self.predicate.clone(), truth(true));
    }
    pub fn bind(&mut self, value: Term) {
        self.predicate = Term::Let {
            value: Box::new(value),
            body: Box::new(self.predicate.clone()),
        };
    }
}

/// All candidates are closed, generated universal judgments. A balanced
/// conjunction avoids spending binder depth linearly on the number of calls.
/// This constructs a candidate only; neither metadata nor pairing verifies it.
pub(super) fn bundle(candidates: &[Candidate]) -> Certificate {
    assert!(
        !candidates.is_empty(),
        "selected contract contributes a judgment"
    );
    if candidates.len() == 1 {
        return candidates[0].certificate.clone();
    }
    let middle = candidates.len() / 2;
    let left = bundle(&candidates[..middle]);
    let right = bundle(&candidates[middle..]);
    Certificate {
        term: Term::pair(left.term, right.term),
        claimed_type: Term::sigma(left.claimed_type, right.claimed_type),
        metadata: Default::default(),
    }
}
