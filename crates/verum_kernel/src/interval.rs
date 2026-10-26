//! The free De Morgan interval algebra, separate from endpoint cofibrations.
//! In particular `i ∧ ~i` is not zero and `i ∨ ~i` is not one. Canonical
//! distributive normal forms retain both a dimension and its involution.
//! Interval expressions are not terms of a fibrant universe.

use crate::{FaceFormula, antichain};
use std::collections::BTreeSet;
use verum_common::{List, Text};

/// An interval generator or its involution, not an endpoint constraint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntervalLiteral {
    /// Dimension name in the separate interval namespace.
    pub dimension: Text,
    /// Whether the interval involution is applied.
    pub reversed: bool,
}

/// Canonical finite De Morgan expression. Private storage preserves the
/// sorted minimal-DNF invariant across every public constructor and operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalExpr {
    clauses: List<BTreeSet<IntervalLiteral>>,
}

impl IntervalExpr {
    /// The zero endpoint.
    pub fn zero() -> Self {
        Self {
            clauses: List::new(),
        }
    }

    /// The one endpoint.
    pub fn one() -> Self {
        Self {
            clauses: [BTreeSet::new()].into_iter().collect(),
        }
    }

    /// A dimension expression. Its scope is checked separately by the context.
    pub fn variable(dimension: impl Into<Text>) -> Self {
        Self::literal(IntervalLiteral {
            dimension: dimension.into(),
            reversed: false,
        })
    }

    fn literal(literal: IntervalLiteral) -> Self {
        Self {
            clauses: [[literal].into_iter().collect()].into_iter().collect(),
        }
    }

    fn from_clauses(clauses: impl IntoIterator<Item = BTreeSet<IntervalLiteral>>) -> Self {
        Self {
            clauses: antichain::minimal(clauses, |left, right| right.is_subset(left)),
        }
    }

    /// Read-only canonical clauses, each a meet of interval literals.
    pub fn clauses(&self) -> &[BTreeSet<IntervalLiteral>] {
        &self.clauses
    }

    /// Whether the expression is definitionally zero.
    pub fn is_zero(&self) -> bool {
        self.clauses.is_empty()
    }

    /// Whether the expression is definitionally one.
    pub fn is_one(&self) -> bool {
        self.clauses.first().is_some_and(BTreeSet::is_empty)
    }

    /// Interval join, with absorption and idempotence.
    pub fn join(&self, other: &Self) -> Self {
        Self::from_clauses(self.clauses.iter().chain(other.clauses.iter()).cloned())
    }

    /// Interval meet. Opposite literals are retained: this is not a Boolean
    /// algebra and not the face algebra's contradictory endpoint conjunction.
    pub fn meet(&self, other: &Self) -> Self {
        Self::from_clauses(self.clauses.iter().flat_map(|left| {
            other
                .clauses
                .iter()
                .map(move |right| left.union(right).cloned().collect())
        }))
    }

    /// Order-reversing involution, distributed by the De Morgan laws.
    pub fn reverse(&self) -> Self {
        self.clauses.iter().fold(Self::one(), |outer, clause| {
            let inner = clause.iter().fold(Self::zero(), |inner, lit| {
                inner.join(&Self::literal(IntervalLiteral {
                    dimension: lit.dimension.clone(),
                    reversed: !lit.reversed,
                }))
            });
            outer.meet(&inner)
        })
    }

    /// Simultaneously replace a dimension by an interval expression. The
    /// replacement is not recursively substituted into itself.
    pub fn substitute(&self, dimension: &str, replacement: &Self) -> Self {
        if !self
            .clauses
            .iter()
            .flatten()
            .any(|lit| lit.dimension.as_str() == dimension)
        {
            return self.clone();
        }
        let reversed = std::cell::OnceCell::new();
        self.clauses.iter().fold(Self::zero(), |outer, clause| {
            let inner = clause.iter().fold(Self::one(), |inner, lit| {
                if lit.dimension.as_str() == dimension {
                    inner.meet(if lit.reversed {
                        reversed.get_or_init(|| replacement.reverse())
                    } else {
                        replacement
                    })
                } else {
                    inner.meet(&Self::literal(lit.clone()))
                }
            });
            outer.join(&inner)
        })
    }

    /// Dimensions occurring in the normal form.
    pub fn dimensions(&self) -> BTreeSet<Text> {
        self.clauses
            .iter()
            .flat_map(|clause| clause.iter().map(|lit| lit.dimension.clone()))
            .collect()
    }

    /// Restrict an interval expression to one consistent conjunction of
    /// endpoint constraints. Dimensions absent from the clause remain free.
    pub fn restrict(&self, clause: &crate::Clause) -> Self {
        clause
            .literals()
            .iter()
            .fold(self.clone(), |expression, literal| {
                expression.substitute(
                    literal.var.as_str(),
                    &if literal.end {
                        Self::one()
                    } else {
                        Self::zero()
                    },
                )
            })
    }

    /// Equality under a face assumption requires free De Morgan equality
    /// after restriction to every disjunct. An empty face is vacuous; merely
    /// agreeing on both endpoints of a free dimension is not sufficient.
    pub fn equivalent_on(&self, other: &Self, face: &FaceFormula) -> bool {
        face.clauses()
            .iter()
            .all(|clause| self.restrict(clause) == other.restrict(clause))
    }

    /// The face where this expression equals the selected endpoint.
    /// Meet/join interchange when projecting the zero endpoint; involuted
    /// generators switch endpoints. This is the bridge to the face algebra.
    pub fn endpoint_face(&self, one: bool) -> FaceFormula {
        self.clauses.iter().fold(
            if one {
                FaceFormula::bottom()
            } else {
                FaceFormula::top()
            },
            |outer, clause| {
                let inner = clause.iter().fold(
                    if one {
                        FaceFormula::top()
                    } else {
                        FaceFormula::bottom()
                    },
                    |inner, lit| {
                        let face = if one != lit.reversed {
                            FaceFormula::at_one(lit.dimension.clone())
                        } else {
                            FaceFormula::at_zero(lit.dimension.clone())
                        };
                        if one {
                            inner.and(&face)
                        } else {
                            inner.or(&face)
                        }
                    },
                );
                if one {
                    outer.or(&inner)
                } else {
                    outer.and(&inner)
                }
            },
        )
    }
}
