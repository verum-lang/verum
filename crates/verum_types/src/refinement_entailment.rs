//! Conservative entailment for total predicates over intrinsic integer sorts.
//!
//! Supported syntax is Boolean literals, `&&`, `||`, and comparisons between
//! the recorded refinement binder and an unsuffixed integer literal exactly
//! representable by both canonical `Int` and the subject's integer sort. Names are
//! resolved only against that binder; calls, fields, arithmetic on the binder,
//! other comparison domains and unknown atoms are refused even when their ASTs
//! are identical. Boolean constants and their combinations are domain-independent.
//!
//! The trusted projection alpha-renames the recorded binder and preserves each
//! comparison's operator and exact literal value in i128. A source atom may be conjoined
//! with weaker query atoms, with a checked [`BoundImplication`] for each addition.
//! This is an equivalent predicate, not an assumption about unrelated atoms.
//! The equality-only lattice certificate independently verifies the Boolean
//! implication on that projected graph. It does NOT prove the arithmetic rules
//! or the AST projection: those are separate, explicit trust obligations here.
//! `false` means unproved (including unsupported syntax or exhausted limits),
//! never a counterexample or permission to fall back to an optimistic rule.

use crate::{
    refinement::{RefinementBinding, RefinementPredicate},
    ty::Type,
    type_algebra::{self, AtomId, Expression, ExpressionDag, ExpressionId, Limits, Outcome},
};
use verum_ast::{
    Expr,
    expr::{BinOp, ExprKind, UnOp},
    literal::LiteralKind,
    ty::PathSegment,
};
use verum_common::List;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// A comparison on the one alpha-normalized, canonical integer binder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegerComparison {
    pub relation: Relation,
    pub bound: i128,
}

/// An arithmetic premise of the Boolean projection, independently inspectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundImplication {
    pub source: IntegerComparison,
    pub target: IntegerComparison,
}

impl BoundImplication {
    /// Sound sufficient rules over mathematical integers. Literal endpoints are
    /// i128, but the binder is not assumed bounded by i128::MIN/MAX. `abs_diff`
    /// tests adjacent bounds without overflowing or truncating either endpoint.
    pub fn check(&self) -> bool {
        use Relation::*;
        let a = self.source.bound;
        let b = self.target.bound;
        match (self.source.relation, self.target.relation) {
            (Eq, Eq) | (Ne, Ne) => a == b,
            (Eq, Ne) => a != b,
            (Eq, Lt) => a < b,
            (Eq, Le) => a <= b,
            (Eq, Gt) => a > b,
            (Eq, Ge) => a >= b,
            (Gt, Gt) | (Ge, Ge) => a >= b,
            (Gt, Ge) => a >= b || a.abs_diff(b) == 1,
            (Ge, Gt) => a > b,
            (Lt, Lt) | (Le, Le) => a <= b,
            (Lt, Le) => a <= b || a.abs_diff(b) == 1,
            (Le, Lt) => a < b,
            (Gt, Ne) => a >= b,
            (Ge, Ne) => a > b,
            (Lt, Ne) => a <= b,
            (Le, Ne) => a < b,
            _ => false,
        }
    }
}

pub fn entails(
    source_base: &Type,
    source: &RefinementPredicate,
    target_base: &Type,
    target: &RefinementPredicate,
) -> bool {
    entails_with_limits(source_base, source, target_base, target, Limits::default())
}

pub fn entails_with_limits(
    source_base: &Type,
    source: &RefinementPredicate,
    target_base: &Type,
    target: &RefinementPredicate,
    limits: Limits,
) -> bool {
    if source_base != target_base {
        return false;
    }
    let mut projection = Projection {
        dag: ExpressionDag::new(limits.max_expression_nodes),
        atoms: List::new(),
        remaining_syntax: limits.max_expression_nodes,
        max_depth: limits.max_depth,
        integer_range: comparison_range(source_base),
    };
    let Some(source_id) = projection.predicate(source) else {
        return false;
    };
    let Some(target_id) = projection.predicate(target) else {
        return false;
    };
    let Some((expanded_source, arithmetic)) = projection.expand_source(source_id) else {
        return false;
    };
    if !arithmetic.iter().all(BoundImplication::check) {
        return false;
    }
    match type_algebra::prove(&projection.dag, expanded_source, target_id, limits) {
        Outcome::Proven(certificate) => type_algebra::check(
            &projection.dag,
            expanded_source,
            target_id,
            &certificate,
            limits,
        )
        .is_ok(),
        Outcome::Undetermined(_) => false,
    }
}

struct Projection {
    dag: ExpressionDag,
    atoms: List<IntegerComparison>,
    remaining_syntax: usize,
    max_depth: usize,
    integer_range: Option<std::ops::RangeInclusive<i128>>,
}

/// The integer-order certificate applies to every subset of the integers.
/// Only interpret a literal when its value is preserved in the selected sort.
fn comparison_range(base: &Type) -> Option<std::ops::RangeInclusive<i128>> {
    use verum_common::well_known_types::type_names;
    let name = match base {
        Type::Int => type_names::INT,
        Type::Named { path, args } if args.is_empty() => path.as_ident()?.as_str(),
        _ => return None,
    };
    let min = type_names::integer_limit(name, "MIN")?;
    // Every source literal admitted by integer_literal fits in i64, including
    // when the carrier's unsigned 128-bit upper bound cannot be stored in i128.
    let max = type_names::integer_limit(name, "MAX")
        .or_else(|| (type_names::is_unsigned_integer_type(name)
            && type_names::numeric_bit_width(name) == Some(128)).then_some(i128::MAX))?;
    Some(min..=max)
}

impl Projection {
    fn predicate(&mut self, predicate: &RefinementPredicate) -> Option<ExpressionId> {
        if matches!(predicate.binding, RefinementBinding::Named(_)) {
            return None;
        }
        enum Work<'a> {
            Enter(&'a Expr, usize),
            Combine(BinOp),
        }
        let mut work = List::from_single(Work::Enter(&predicate.predicate, 1));
        let mut values = List::new();
        while let Some(item) = work.pop() {
            match item {
                Work::Enter(expr, depth) => {
                    if depth > self.max_depth || self.remaining_syntax == 0 {
                        return None;
                    }
                    self.remaining_syntax -= 1;
                    let id = match &expr.kind {
                        ExprKind::Paren(inner) => {
                            if depth >= self.max_depth {
                                return None;
                            }
                            work.push(Work::Enter(inner, depth + 1));
                            continue;
                        }
                        ExprKind::Literal(literal) => match literal.kind {
                            LiteralKind::Bool(true) => self.dag.top().ok()?,
                            LiteralKind::Bool(false) => self.dag.bottom().ok()?,
                            _ => return None,
                        },
                        ExprKind::Binary {
                            op: op @ (BinOp::And | BinOp::Or),
                            left,
                            right,
                        } => {
                            if depth >= self.max_depth {
                                return None;
                            }
                            work.push(Work::Combine(*op));
                            work.push(Work::Enter(right, depth + 1));
                            work.push(Work::Enter(left, depth + 1));
                            continue;
                        }
                        ExprKind::Binary { op, left, right } => {
                            let atom = comparison(*op, left, right, predicate)?;
                            if !self.integer_range.as_ref()?.contains(&atom.bound) { return None; }
                            let index = match self.atoms.iter().position(|known| *known == atom) {
                                Some(index) => index,
                                None => {
                                    let index = self.atoms.len();
                                    self.atoms.push(atom);
                                    index
                                }
                            };
                            self.dag.atom(AtomId(u32::try_from(index).ok()?)).ok()?
                        }
                        _ => return None,
                    };
                    values.push(id);
                }
                Work::Combine(op) => {
                    let right = values.pop()?;
                    let left = values.pop()?;
                    values.push(match op {
                        BinOp::And => self.dag.intersection(left, right).ok()?,
                        BinOp::Or => self.dag.union(left, right).ok()?,
                        _ => return None,
                    });
                }
            }
        }
        if values.len() == 1 {
            values.pop()
        } else {
            None
        }
    }

    fn expand_source(
        &mut self,
        source: ExpressionId,
    ) -> Option<(ExpressionId, List<BoundImplication>)> {
        let mut rewritten = List::new();
        let mut arithmetic = List::new();
        // DAG children precede parents; preserve the original target graph.
        for index in 0..=source.0 {
            let original = ExpressionId(index);
            let replacement = match *self.dag.expression(original)? {
                Expression::Atom(AtomId(atom)) => {
                    let source_atom = *self.atoms.get(atom as usize)?;
                    let mut expanded = original;
                    for (target_index, target_atom) in self.atoms.iter().enumerate() {
                        let witness = BoundImplication {
                            source: source_atom,
                            target: *target_atom,
                        };
                        if source_atom != *target_atom && witness.check() {
                            let target = self
                                .dag
                                .atom(AtomId(u32::try_from(target_index).ok()?))
                                .ok()?;
                            expanded = self.dag.intersection(expanded, target).ok()?;
                            arithmetic.push(witness);
                        }
                    }
                    expanded
                }
                Expression::Intersection(left, right) => self
                    .dag
                    .intersection(*rewritten.get(left.0)?, *rewritten.get(right.0)?)
                    .ok()?,
                Expression::Union(left, right) => self
                    .dag
                    .union(*rewritten.get(left.0)?, *rewritten.get(right.0)?)
                    .ok()?,
                Expression::Top | Expression::Bottom => original,
            };
            rewritten.push(replacement);
        }
        Some((*rewritten.get(source.0)?, arithmetic))
    }
}

fn comparison(
    op: BinOp,
    left: &Expr,
    right: &Expr,
    predicate: &RefinementPredicate,
) -> Option<IntegerComparison> {
    use Relation::*;
    let relation = match op {
        BinOp::Eq => Eq,
        BinOp::Ne => Ne,
        BinOp::Lt => Lt,
        BinOp::Le => Le,
        BinOp::Gt => Gt,
        BinOp::Ge => Ge,
        _ => return None,
    };
    let is_binder = |expr: &Expr| match &expr.kind {
        ExprKind::Path(path) => match path.segments.as_slice() {
            [PathSegment::Name(ident)] => predicate.binds_subject(ident.name.as_str()),
            [PathSegment::SelfValue] => predicate.binds_subject("self"),
            _ => false,
        },
        _ => false,
    };
    if is_binder(left) {
        Some(IntegerComparison {
            relation,
            bound: integer_literal(right)?,
        })
    } else if is_binder(right) {
        let relation = match relation {
            Eq => Eq,
            Ne => Ne,
            Lt => Gt,
            Le => Ge,
            Gt => Lt,
            Ge => Le,
        };
        Some(IntegerComparison {
            relation,
            bound: integer_literal(left)?,
        })
    } else {
        None
    }
}

fn integer_literal(expr: &Expr) -> Option<i128> {
    let direct = |expr: &Expr| match &expr.kind {
        ExprKind::Literal(literal) => match &literal.kind {
            LiteralKind::Int(integer) if integer.suffix.is_none() => Some(integer.value),
            _ => None,
        },
        _ => None,
    };
    let value = match &expr.kind {
        ExprKind::Unary {
            op: UnOp::Neg,
            expr,
        } => direct(expr)?.checked_neg(),
        _ => direct(expr),
    }?;
    // Validate after negation so the literal spelling of Int::MIN remains
    // expressible, without accepting wider AST literals that runtime Int
    // cannot represent exactly.
    i64::try_from(value).ok().map(i128::from)
}
