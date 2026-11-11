//! Refinement-Aware Exhaustiveness
//!
//! This module enhances exhaustiveness checking by leveraging refinement types.
//! When the scrutinee has a refinement type, we can eliminate impossible cases
//! and provide more precise exhaustiveness analysis.
//!
//! ## Example
//!
//! ```verum
//! type Positive = Int{x: x > 0};
//!
//! fn classify(n: Positive) -> Text =
//!  match n {
//!  1 => "one",
//!  2 => "two",
//!  _ => "many" // Wildcard only covers positive integers
//!  }
//! ```
//!
//! Without refinement awareness, we might warn that negative integers aren't
//! covered. With refinement awareness, we know those cases are impossible.
//!
//! ## Integration
//!
//! This module integrates with:
//! - `verum_smt` for constraint solving
//! - `crate::ty::Type::Refined` for refinement type information
//! - The main exhaustiveness checker for eliminating impossible constructors

use super::constructors::Constructor;
use super::matrix::CoverageMatrix;
use super::ranges::{Interval, IntervalSet};
use super::witness::Witness;
use crate::TypeError;
use crate::context::TypeEnv;
use crate::refinement::RefinementPredicate;
use crate::ty::Type;
use verum_ast::expr::{BinOp, Expr, ExprKind, UnOp};
use verum_common::{List, Maybe, Text};

/// Refinement constraint extracted from a refinement type
#[derive(Debug, Clone)]
pub enum RefinementConstraint {
    /// x > value
    GreaterThan(i128),
    /// x >= value
    GreaterOrEqual(i128),
    /// x < value
    LessThan(i128),
    /// x <= value
    LessOrEqual(i128),
    /// x == value
    Equal(i128),
    /// x != value
    NotEqual(i128),
    /// x in [start, end]
    InRange(i128, i128),
    /// Conjunction of constraints
    And(Box<RefinementConstraint>, Box<RefinementConstraint>),
    /// Disjunction of constraints
    Or(Box<RefinementConstraint>, Box<RefinementConstraint>),
    /// Negation
    Not(Box<RefinementConstraint>),
    /// Constraint on enum variant
    IsVariant(Text),
    /// No constraint (always true)
    True,
    /// Impossible (always false)
    False,
}

impl RefinementConstraint {
    /// Convert constraint to an interval set for numeric types
    pub fn to_interval_set(&self) -> IntervalSet {
        match self {
            RefinementConstraint::GreaterThan(v) => v
                .checked_add(1)
                .map(|start| IntervalSet::singleton(Interval::new(start, i128::MAX)))
                .unwrap_or_else(IntervalSet::empty),
            RefinementConstraint::GreaterOrEqual(v) => {
                IntervalSet::singleton(Interval::new(*v, i128::MAX))
            }
            RefinementConstraint::LessThan(v) => v
                .checked_sub(1)
                .map(|end| IntervalSet::singleton(Interval::new(i128::MIN, end)))
                .unwrap_or_else(IntervalSet::empty),
            RefinementConstraint::LessOrEqual(v) => {
                IntervalSet::singleton(Interval::new(i128::MIN, *v))
            }
            RefinementConstraint::Equal(v) => IntervalSet::singleton(Interval::singleton(*v)),
            RefinementConstraint::InRange(start, end) => {
                IntervalSet::singleton(Interval::new(*start, *end))
            }
            RefinementConstraint::And(a, b) => {
                // Intersection
                let set_a = a.to_interval_set();
                let set_b = b.to_interval_set();
                set_a.intersect(&set_b)
            }
            RefinementConstraint::Or(a, b) => {
                // Union
                let mut result = a.to_interval_set();
                result.union_with(&b.to_interval_set());
                result
            }
            RefinementConstraint::NotEqual(v) => {
                // Everything except v
                let mut result = IntervalSet::universe();
                result.subtract(Interval::singleton(*v));
                result
            }
            RefinementConstraint::Not(inner) => {
                // Complement
                let inner_set = inner.to_interval_set();
                inner_set.complement(Interval::new(i128::MIN, i128::MAX))
            }
            RefinementConstraint::True => IntervalSet::universe(),
            RefinementConstraint::False | RefinementConstraint::IsVariant(_) => {
                IntervalSet::empty()
            }
        }
    }

    /// Check if a value satisfies this constraint
    pub fn satisfies(&self, value: i128) -> bool {
        match self {
            RefinementConstraint::GreaterThan(v) => value > *v,
            RefinementConstraint::GreaterOrEqual(v) => value >= *v,
            RefinementConstraint::LessThan(v) => value < *v,
            RefinementConstraint::LessOrEqual(v) => value <= *v,
            RefinementConstraint::Equal(v) => value == *v,
            RefinementConstraint::NotEqual(v) => value != *v,
            RefinementConstraint::InRange(start, end) => value >= *start && value <= *end,
            RefinementConstraint::And(a, b) => a.satisfies(value) && b.satisfies(value),
            RefinementConstraint::Or(a, b) => a.satisfies(value) || b.satisfies(value),
            RefinementConstraint::Not(inner) => !inner.satisfies(value),
            RefinementConstraint::True => true,
            RefinementConstraint::False | RefinementConstraint::IsVariant(_) => false,
        }
    }

    /// Combine two constraints with AND
    pub fn and(self, other: Self) -> Self {
        match (&self, &other) {
            (RefinementConstraint::True, _) => other,
            (_, RefinementConstraint::True) => self,
            (RefinementConstraint::False, _) | (_, RefinementConstraint::False) => {
                RefinementConstraint::False
            }
            _ => RefinementConstraint::And(Box::new(self), Box::new(other)),
        }
    }

    /// Combine two constraints with OR
    pub fn or(self, other: Self) -> Self {
        match (&self, &other) {
            (RefinementConstraint::False, _) => other,
            (_, RefinementConstraint::False) => self,
            (RefinementConstraint::True, _) | (_, RefinementConstraint::True) => {
                RefinementConstraint::True
            }
            _ => RefinementConstraint::Or(Box::new(self), Box::new(other)),
        }
    }
}

/// Extract refinement constraint from a refined type
pub fn extract_refinement(ty: &Type) -> Option<RefinementConstraint> {
    match ty {
        Type::Refined { base, predicate } => {
            let outer = extract_constraint_from_expr(&predicate.predicate, predicate)?;
            let inner = match base.as_ref() {
                Type::Int => RefinementConstraint::True,
                Type::Refined { .. } => extract_refinement(base)?,
                _ => return None,
            };
            Some(inner.and(outer))
        }
        _ => None,
    }
}

fn is_subject(expr: &Expr, predicate: &RefinementPredicate) -> bool {
    match &expr.kind {
        ExprKind::Path(path) => path
            .as_ident()
            .is_some_and(|name| predicate.binds_subject(name.as_str())),
        ExprKind::Paren(inner) => is_subject(inner, predicate),
        _ => false,
    }
}

/// Lower only comparisons whose operand is this predicate's actual subject.
/// Reject an unsupported expression as a whole: under negation, replacing an
/// unknown subexpression by True or False would unsoundly narrow the domain.
fn extract_constraint_from_expr(
    expr: &Expr,
    predicate: &RefinementPredicate,
) -> Option<RefinementConstraint> {
    use verum_ast::literal::LiteralKind;
    match &expr.kind {
        ExprKind::Binary {
            op: BinOp::And,
            left,
            right,
        } => Some(
            extract_constraint_from_expr(left, predicate)?
                .and(extract_constraint_from_expr(right, predicate)?),
        ),
        ExprKind::Binary {
            op: BinOp::Or,
            left,
            right,
        } => Some(
            extract_constraint_from_expr(left, predicate)?
                .or(extract_constraint_from_expr(right, predicate)?),
        ),
        ExprKind::Binary { op, left, right } => {
            let (op, value) = if is_subject(left, predicate) {
                (*op, extract_int_literal(right)?)
            } else if is_subject(right, predicate) {
                let reversed = match op {
                    BinOp::Lt => BinOp::Gt,
                    BinOp::Le => BinOp::Ge,
                    BinOp::Gt => BinOp::Lt,
                    BinOp::Ge => BinOp::Le,
                    BinOp::Eq => BinOp::Eq,
                    BinOp::Ne => BinOp::Ne,
                    _ => return None,
                };
                (reversed, extract_int_literal(left)?)
            } else {
                return None;
            };
            match op {
                BinOp::Lt => Some(RefinementConstraint::LessThan(value)),
                BinOp::Le => Some(RefinementConstraint::LessOrEqual(value)),
                BinOp::Gt => Some(RefinementConstraint::GreaterThan(value)),
                BinOp::Ge => Some(RefinementConstraint::GreaterOrEqual(value)),
                BinOp::Eq => Some(RefinementConstraint::Equal(value)),
                BinOp::Ne => Some(RefinementConstraint::NotEqual(value)),
                _ => None,
            }
        }
        ExprKind::Unary {
            op: UnOp::Not,
            expr: inner,
        } => Some(RefinementConstraint::Not(Box::new(
            extract_constraint_from_expr(inner, predicate)?,
        ))),
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Bool(true) => Some(RefinementConstraint::True),
            LiteralKind::Bool(false) => Some(RefinementConstraint::False),
            _ => None,
        },
        ExprKind::Paren(inner) => extract_constraint_from_expr(inner, predicate),
        _ => None,
    }
}

/// Extract integer literal value from expression
fn extract_int_literal(expr: &Expr) -> Option<i128> {
    use verum_ast::literal::LiteralKind;

    match &expr.kind {
        ExprKind::Literal(lit) => match &lit.kind {
            LiteralKind::Int(int_lit) => Some(int_lit.value),
            _ => None,
        },
        ExprKind::Paren(inner) => extract_int_literal(inner),
        ExprKind::Unary { op, expr: inner } => match op {
            UnOp::Neg => extract_int_literal(inner).and_then(i128::checked_neg),
            _ => None,
        },
        _ => None,
    }
}

/// Refinement-aware exhaustiveness context
pub struct RefinementContext {
    /// The refinement constraint on the scrutinee
    constraint: RefinementConstraint,
    /// Valid interval set for numeric scrutinee
    valid_intervals: IntervalSet,
}

impl RefinementContext {
    /// Create a new refinement context from a type
    pub fn from_type(ty: &Type) -> Option<Self> {
        let constraint = extract_refinement(ty)?;
        let valid_intervals = constraint
            .to_interval_set()
            .intersect(&IntervalSet::singleton(super::scalar::INT_BOUNDS));
        Some(Self {
            constraint,
            valid_intervals,
        })
    }

    /// Check if a constructor is possible under the refinement
    pub fn is_constructor_possible(&self, ctor: &Constructor) -> bool {
        // For variant constraints
        if let RefinementConstraint::IsVariant(ref variant_name) = self.constraint {
            return &ctor.name == variant_name;
        }
        true
    }

    /// Check if a literal value is possible under the refinement
    pub fn is_literal_possible(&self, value: i128) -> bool {
        self.valid_intervals.contains(value)
    }

    /// Get the refined interval set
    pub fn valid_intervals(&self) -> &IntervalSet {
        &self.valid_intervals
    }

    /// Filter constructors to only those possible under refinement
    pub fn filter_constructors(
        &self,
        ctors: &super::constructors::TypeConstructors,
    ) -> List<Constructor> {
        ctors
            .iter()
            .filter(|c| self.is_constructor_possible(c))
            .cloned()
            .collect()
    }

    /// Generate a witness that satisfies the refinement
    pub fn generate_valid_witness(&self) -> Option<Witness> {
        self.valid_intervals.iter().next().map(|interval| {
            Witness::int(
                i64::try_from(interval.start).expect("refinement is clipped to its Int carrier"),
            )
        })
    }
}

/// Exact coverage for the supported integer predicate fragment. For an
/// unsupported predicate, only coverage of the entire base type establishes
/// success; a base-type gap is not necessarily inhabited by the refinement.
pub fn check_exhaustiveness_with_refinement(
    matrix: &CoverageMatrix,
    scrutinee_ty: &Type,
    env: &TypeEnv,
) -> Result<(bool, List<Witness>), TypeError> {
    let Some(context) = RefinementContext::from_type(scrutinee_ty) else {
        let mut base = scrutinee_ty;
        while let Type::Refined { base: inner, .. } = base {
            base = inner;
        }
        let uncovered = super::find_uncovered_cases(matrix, base, env)?;
        return Ok((uncovered.is_empty(), List::new()));
    };
    let covered = super::scalar::covered_integers(
        matrix
            .rows
            .iter()
            .filter(|row| !row.has_guard)
            .filter_map(|row| row.columns.first()),
    );
    let uncovered: List<_> = context
        .valid_intervals()
        .iter()
        .filter_map(|bounds| covered.first_uncovered(*bounds))
        .map(|value| Witness::int(i64::try_from(value).expect("refinement witness lies in Int")))
        .collect();
    Ok((uncovered.is_empty(), uncovered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constraint_greater_than() {
        let constraint = RefinementConstraint::GreaterThan(0);
        assert!(!constraint.satisfies(-1));
        assert!(!constraint.satisfies(0));
        assert!(constraint.satisfies(1));
        assert!(constraint.satisfies(100));
    }

    #[test]
    fn test_constraint_in_range() {
        let constraint = RefinementConstraint::InRange(1, 10);
        assert!(!constraint.satisfies(0));
        assert!(constraint.satisfies(1));
        assert!(constraint.satisfies(5));
        assert!(constraint.satisfies(10));
        assert!(!constraint.satisfies(11));
    }

    #[test]
    fn test_constraint_and() {
        let c1 = RefinementConstraint::GreaterThan(0);
        let c2 = RefinementConstraint::LessThan(10);
        let combined = c1.and(c2);

        assert!(!combined.satisfies(0));
        assert!(combined.satisfies(5));
        assert!(!combined.satisfies(10));
    }

    #[test]
    fn test_constraint_or() {
        let c1 = RefinementConstraint::Equal(1);
        let c2 = RefinementConstraint::Equal(2);
        let combined = c1.or(c2);

        assert!(!combined.satisfies(0));
        assert!(combined.satisfies(1));
        assert!(combined.satisfies(2));
        assert!(!combined.satisfies(3));
    }

    #[test]
    fn test_constraint_to_interval_set() {
        let constraint = RefinementConstraint::GreaterOrEqual(1);
        let intervals = constraint.to_interval_set();
        assert!(intervals.contains(1));
        assert!(intervals.contains(100));
        assert!(!intervals.contains(0));
    }

    #[test]
    fn test_true_false_constraints() {
        assert!(RefinementConstraint::True.satisfies(42));
        assert!(!RefinementConstraint::False.satisfies(42));
    }

    #[test]
    fn test_not_constraint() {
        let positive = RefinementConstraint::GreaterThan(0);
        let not_positive = RefinementConstraint::Not(Box::new(positive));

        assert!(not_positive.satisfies(-5));
        assert!(not_positive.satisfies(0));
        assert!(!not_positive.satisfies(1));
    }
}
