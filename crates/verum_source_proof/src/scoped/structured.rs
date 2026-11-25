//! Submit structured proofs to the same scoped goal interpreter as single tactics.
use super::{tactics::Action, *};
use verum_ast::decl::ProofStructure;
use verum_common::List;

impl Elab {
    pub(super) fn structured(&mut self, block: &ProofStructure, goal: &Ir) -> Result<Ir> {
        let mut pending = List::new();
        if let Maybe::Some(last) = &block.conclusion {
            pending.push(Action::Tactic(last));
        }
        pending.extend(block.steps.iter().rev().map(Action::Step));
        self.prove_with_actions(pending, goal).map_err(|e| self.locate(e, block.span))
    }
}
