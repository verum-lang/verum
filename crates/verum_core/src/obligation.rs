//! Proof obligations.
//!
//! An [`IrObligation`] captures what the solver has to decide: a list
//! of hypotheses and a goal, plus provenance metadata for diagnostic
//! reporting and certificate export. It's the shared intermediate
//! between hypothesis elaboration (in `verum_compiler`) and solver
//! dispatch (in `verum_smt`).

use serde::{Deserialize, Serialize};
use verum_common::{List, Maybe, Text};
use verum_common::span::Span;

use crate::expr::IrExpr;

/// A single proof obligation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrObligation {
    /// Optional name for diagnostic tagging.
    pub name: Maybe<Text>,
    /// Hypotheses (facts the solver may assume).
    pub hypotheses: List<IrExpr>,
    /// The goal (what the solver must show).
    pub goal: IrExpr,
    /// Provenance: why the obligation exists.
    pub provenance: ObligationProvenance,
    /// Source span of the obligation's originating declaration.
    pub span: Span,
}

/// Why an obligation was emitted. Used for diagnostic grouping,
/// certificate layout, and cache keying.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ObligationProvenance {
    /// Directly from a user-written `ensures` clause.
    EnsuresClause {
        /// Name of the enclosing function / theorem.
        owner: Text,
        /// Zero-based index within the owner's ensures list.
        index: u32,
    },
    /// Directly from a user-written `requires` clause — checked for
    /// satisfiability so contradictory preconditions surface as
    /// diagnostics rather than silent vacuous proofs.
    RequiresClause {
        /// Name of the enclosing function / theorem.
        owner: Text,
        /// Zero-based index within the owner's requires list.
        index: u32,
    },
    /// An internally-generated subgoal (tactic fan-out, case split,
    /// structured-proof step).
    Subgoal {
        /// Parent obligation's name (if any).
        parent: Maybe<Text>,
        /// Tactic that produced the subgoal.
        origin: Text,
    },
    /// Refinement-type checking obligation.
    RefinementCheck {
        /// Which parameter / variable the refinement applies to.
        owner: Text,
    },
    /// Frame-axiom obligation (`variant_disjointness`, stdlib axioms).
    /// Not actually checked — asserted as a hypothesis — but tracked
    /// for certificate completeness.
    Frame {
        /// Descriptive tag for the frame family.
        tag: Text,
    },
}

impl IrObligation {
    /// Build a fresh obligation with no hypotheses.
    #[must_use]
    pub fn new(goal: IrExpr, provenance: ObligationProvenance, span: Span) -> Self {
        Self {
            name: Maybe::None,
            hypotheses: List::new(),
            goal,
            provenance,
            span,
        }
    }

    /// Extend the hypothesis set in place.
    pub fn add_hypothesis(&mut self, hyp: IrExpr) {
        self.hypotheses.push(hyp);
    }

    /// Number of hypotheses.
    #[must_use]
    pub fn hypothesis_count(&self) -> usize {
        self.hypotheses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::expr::{IrExpr, IrExprKind};
    use verum_common::span::Span;

    #[test]
    fn obligation_extends() {
        let goal = IrExpr::new(IrExprKind::BoolLit(true), None, Span::dummy());
        let prov = ObligationProvenance::Subgoal {
            parent: Maybe::None,
            origin: Text::from("auto"),
        };
        let mut o = IrObligation::new(goal, prov, Span::dummy());
        assert_eq!(o.hypothesis_count(), 0);
        o.add_hypothesis(IrExpr::new(IrExprKind::BoolLit(true), None, Span::dummy()));
        assert_eq!(o.hypothesis_count(), 1);
    }
}
