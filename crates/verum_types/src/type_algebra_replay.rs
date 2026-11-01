//! Export an exact lattice query and its checked derivation for Lean replay.
//!
//! The output is self-contained. Every proof step uses a proved semantic rule;
//! Lean checks context rearrangements and the final endpoints independently.
//! This proves the lattice judgment for opaque atoms, not the caller's mapping
//! from source types to atoms or any runtime representation correspondence.

use std::fmt::{self, Write};

use verum_common::{List, Text};

use crate::type_algebra::{
    self, Certificate, CheckError, Expression, ExpressionDag, ExpressionId, Limits, ProofId, Rule,
};

const SEMANTICS: &str =
    include_str!("../../../verification/external/lean/VerumExternalReplay/TypeAlgebra.lean");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayError {
    InvalidCertificate(CheckError),
    OutputLimit,
}

struct Output {
    text: Text,
    limit: usize,
}

impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() > self.limit.saturating_sub(self.text.len()) {
            return Err(fmt::Error);
        }
        self.text.push_str(text);
        Ok(())
    }
}

struct Context<'a>(&'a [ExpressionId]);

impl fmt::Display for Context<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (index, id) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "e{}", id.0)?;
        }
        f.write_str("]")
    }
}

fn without(context: &[ExpressionId], principal: ExpressionId) -> List<ExpressionId> {
    context
        .iter()
        .copied()
        .filter(|id| *id != principal)
        .collect()
}

fn prepend(prefix: &[ExpressionId], rest: &[ExpressionId]) -> List<ExpressionId> {
    prefix.iter().chain(rest.iter()).copied().collect()
}

fn premise(
    output: &mut Output,
    number: usize,
    proof: ProofId,
    left: &[ExpressionId],
    right: &[ExpressionId],
) -> fmt::Result {
    writeln!(
        output,
        "  have h{number} : Valid {} {} := reframe (by decide) (by decide) p{}",
        Context(left),
        Context(right),
        proof.0,
    )
}

/// Render only a certificate that the native checker accepts for this exact
/// graph and endpoints. `max_bytes` bounds the entire generated source, including
/// the semantic model. Successful export is not a claim that Lean was executed.
pub fn export_lean(
    dag: &ExpressionDag,
    source: ExpressionId,
    target: ExpressionId,
    certificate: &Certificate,
    limits: Limits,
    max_bytes: usize,
) -> Result<Text, ReplayError> {
    type_algebra::check(dag, source, target, certificate, limits)
        .map_err(ReplayError::InvalidCertificate)?;
    let mut output = Output {
        text: Text::new(),
        limit: max_bytes,
    };
    render(&mut output, dag, source, target, certificate).map_err(|_| ReplayError::OutputLimit)?;
    Ok(output.text)
}

fn render(
    out: &mut Output,
    dag: &ExpressionDag,
    source: ExpressionId,
    target: ExpressionId,
    certificate: &Certificate,
) -> fmt::Result {
    out.write_str(SEMANTICS)?;
    out.write_str("\nnamespace Verum.TypeAlgebraReplay\nopen Verum.TypeAlgebra\n")?;
    for (index, expression) in dag.nodes().iter().enumerate() {
        write!(out, "private def e{index} : Formula := ")?;
        match expression {
            Expression::Bottom => writeln!(out, ".bottom")?,
            Expression::Top => writeln!(out, ".top")?,
            Expression::Atom(id) => writeln!(out, ".atom {}", id.0)?,
            Expression::Union(a, b) => writeln!(out, ".union e{} e{}", a.0, b.0)?,
            Expression::Intersection(a, b) => writeln!(out, ".intersection e{} e{}", a.0, b.0)?,
        }
    }
    for (index, step) in certificate.steps.iter().enumerate() {
        let left = &step.sequent.left;
        let right = &step.sequent.right;
        writeln!(
            out,
            "private theorem p{index} : Valid {} {} := by",
            Context(left),
            Context(right)
        )?;
        match step.rule {
            Rule::Identity(id) => {
                writeln!(out, "  exact identity e{} (by decide) (by decide)", id.0)?
            }
            Rule::BottomLeft(_) => writeln!(out, "  exact bottomLeft (by decide)")?,
            Rule::TopRight(_) => writeln!(out, "  exact topRight (by decide)")?,
            Rule::IntersectionLeft(id, proof) => {
                let Expression::Intersection(a, b) = dag.nodes()[id.0] else {
                    return Err(fmt::Error);
                };
                let rest = without(left, id);
                premise(out, 0, proof, &prepend(&[a, b], &rest), right)?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := intersectionLeft h0",
                    Context(&prepend(&[id], &rest)),
                    Context(right)
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
            Rule::UnionRight(id, proof) => {
                let Expression::Union(a, b) = dag.nodes()[id.0] else {
                    return Err(fmt::Error);
                };
                let rest = without(right, id);
                premise(out, 0, proof, left, &prepend(&[a, b], &rest))?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := unionRight h0",
                    Context(left),
                    Context(&prepend(&[id], &rest))
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
            Rule::TopLeft(id, proof) => {
                let rest = without(left, id);
                premise(out, 0, proof, &rest, right)?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := topLeft h0",
                    Context(&prepend(&[id], &rest)),
                    Context(right)
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
            Rule::BottomRight(id, proof) => {
                let rest = without(right, id);
                premise(out, 0, proof, left, &rest)?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := bottomRight h0",
                    Context(left),
                    Context(&prepend(&[id], &rest))
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
            Rule::UnionLeft(id, first, second) => {
                let Expression::Union(a, b) = dag.nodes()[id.0] else {
                    return Err(fmt::Error);
                };
                let rest = without(left, id);
                premise(out, 0, first, &prepend(&[a], &rest), right)?;
                premise(out, 1, second, &prepend(&[b], &rest), right)?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := unionLeft h0 h1",
                    Context(&prepend(&[id], &rest)),
                    Context(right)
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
            Rule::IntersectionRight(id, first, second) => {
                let Expression::Intersection(a, b) = dag.nodes()[id.0] else {
                    return Err(fmt::Error);
                };
                let rest = without(right, id);
                premise(out, 0, first, left, &prepend(&[a], &rest))?;
                premise(out, 1, second, left, &prepend(&[b], &rest))?;
                writeln!(
                    out,
                    "  have h : Valid {} {} := intersectionRight h0 h1",
                    Context(left),
                    Context(&prepend(&[id], &rest))
                )?;
                writeln!(out, "  exact reframe (by decide) (by decide) h")?;
            }
        }
    }
    writeln!(
        out,
        "theorem verifiedQuery : ∀ valuation, denote valuation e{} → denote valuation e{} :=\n  entails p{}\n#print axioms verifiedQuery\nend Verum.TypeAlgebraReplay",
        source.0, target.0, certificate.root.0,
    )
}
