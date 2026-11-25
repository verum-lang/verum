//! A local lemma enters scope after its justification has checked.
//! The certificate shares its witness through an ordinary kernel let.
use super::*;
use verum_ast::decl::{ProofStepKind, ProofStructure};

impl Elab {
    pub(super) fn structured(&mut self, block: &ProofStructure, goal: &Ir) -> Result<Ir> {
        let saved = self.stack.len();
        let result = self.structured_inner(block, goal);
        self.stack.truncate(saved);
        result
    }

    fn structured_inner(&mut self, block: &ProofStructure, goal: &Ir) -> Result<Ir> {
        let mut definitions = Vec::new();
        let mut closing = Vec::new();
        for step in &block.steps {
            match &step.kind {
                ProofStepKind::Have {
                    name,
                    proposition,
                    justification,
                } if closing.is_empty() => {
                    let domain = self.expr(proposition)?;
                    self.require_type(&domain)
                        .map_err(|e| self.locate(e, proposition.span))?;
                    let value = self
                        .tactics(justification, &domain)
                        .map_err(|e| self.locate(e, step.span))?;
                    let binding = self.bind(Some(name.as_str().into()), domain, false)?;
                    definitions.push((binding.id, value));
                }
                ProofStepKind::Tactic(tactic) => closing.push(tactic.clone()),
                _ => {
                    return Err(self.locate(
                        unsupported("expected a proved local lemma followed by closing tactics"),
                        step.span,
                    ));
                }
            }
        }
        if let Maybe::Some(last) = &block.conclusion {
            closing.push(last.clone());
        }
        let mut body = self
            .tactics(&TacticExpr::Seq(closing.into_iter().collect()), goal)
            .map_err(|e| self.locate(e, block.span))?;
        for (id, value) in definitions.into_iter().rev() {
            body = Ir::Let {
                id,
                value: Box::new(value),
                body: Box::new(body),
            };
        }
        Ok(body)
    }
}
