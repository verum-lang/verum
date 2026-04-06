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
//!     match n {
//!         1 => "one",
//!         2 => "two",
//!         _ => "many"  // Wildcard only covers positive integers
//!     }
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

use super::constructors::{Constructor, get_type_constructors};
use super::matrix::PatternColumn;
use super::ranges::{Interval, IntervalSet};
use super::witness::Witness;
use crate::context::TypeEnv;
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
            RefinementConstraint::GreaterThan(v) => {
                IntervalSet::singleton(Interval::new(*v + 1, i128::MAX))
            }
            RefinementConstraint::GreaterOrEqual(v) => {
                IntervalSet::singleton(Interval::new(*v, i128::MAX))
            }
            RefinementConstraint::LessThan(v) => {
                IntervalSet::singleton(Interval::new(i128::MIN, *v - 1))
            }
            RefinementConstraint::LessOrEqual(v) => {
                IntervalSet::singleton(Interval::new(i128::MIN, *v))
            }
            RefinementConstraint::Equal(v) => {
                IntervalSet::singleton(Interval::singleton(*v))
            }
            RefinementConstraint::InRange(start, end) => {
                IntervalSet::singleton(Interval::new(*start, *end))
            }
            RefinementConstraint::And(a, b) => {
                // Intersection
                let set_a = a.to_interval_set();
                let set_b = b.to_interval_set();
                intersect_interval_sets(&set_a, &set_b)
            }
            RefinementConstraint::Or(a, b) => {
                // Union
                let mut result = a.to_interval_set();
                for interval in b.to_interval_set().iter() {
                    result.add(*interval);
                }
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
        Type::Refined { base: _, predicate } => {
            // The predicate field contains the expression we need
            extract_constraint_from_expr(&predicate.predicate)
        }
        _ => None,
    }
}

/// Extract constraint from a predicate expression
fn extract_constraint_from_expr(expr: &Expr) -> Option<RefinementConstraint> {
    use verum_ast::literal::LiteralKind;

    match &expr.kind {
        ExprKind::Binary { op, left, right } => {
            // Handle comparison operators
            let rhs_value = extract_int_literal(right);

            match op {
                BinOp::Lt => rhs_value.map(RefinementConstraint::LessThan),
                BinOp::Le => rhs_value.map(RefinementConstraint::LessOrEqual),
                BinOp::Gt => rhs_value.map(RefinementConstraint::GreaterThan),
                BinOp::Ge => rhs_value.map(RefinementConstraint::GreaterOrEqual),
                BinOp::Eq => rhs_value.map(RefinementConstraint::Equal),
                BinOp::Ne => rhs_value.map(RefinementConstraint::NotEqual),
                BinOp::And => {
                    let l = extract_constraint_from_expr(left)?;
                    let r = extract_constraint_from_expr(right)?;
                    Some(l.and(r))
                }
                BinOp::Or => {
                    let l = extract_constraint_from_expr(left)?;
                    let r = extract_constraint_from_expr(right)?;
                    Some(l.or(r))
                }
                _ => None,
            }
        }
        ExprKind::Unary { op, expr: inner } => {
            match op {
                UnOp::Not => {
                    let c = extract_constraint_from_expr(inner)?;
                    Some(RefinementConstraint::Not(Box::new(c)))
                }
                _ => None,
            }
        }
        ExprKind::Literal(lit) => {
            match &lit.kind {
                LiteralKind::Bool(true) => Some(RefinementConstraint::True),
                LiteralKind::Bool(false) => Some(RefinementConstraint::False),
                _ => None,
            }
        }
        ExprKind::Paren(inner) => extract_constraint_from_expr(inner),
        _ => None,
    }
}

/// Extract integer literal value from expression
fn extract_int_literal(expr: &Expr) -> Option<i128> {
    use verum_ast::literal::LiteralKind;

    match &expr.kind {
        ExprKind::Literal(lit) => {
            match &lit.kind {
                LiteralKind::Int(int_lit) => Some(int_lit.value),
                _ => None,
            }
        }
        ExprKind::Paren(inner) => extract_int_literal(inner),
        ExprKind::Unary { op, expr: inner } => {
            match op {
                UnOp::Neg => extract_int_literal(inner).map(|n| -n),
                _ => None,
            }
        }
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
        let valid_intervals = constraint.to_interval_set();
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
    pub fn filter_constructors(&self, ctors: &super::constructors::TypeConstructors) -> List<Constructor> {
        ctors
            .iter()
            .filter(|c| self.is_constructor_possible(c))
            .cloned()
            .collect()
    }

    /// Generate a witness that satisfies the refinement
    pub fn generate_valid_witness(&self) -> Option<Witness> {
        // For numeric refinements, find a value in the valid interval
        for interval in self.valid_intervals.iter() {
            // Prefer small positive values
            if interval.contains(0) {
                return Some(Witness::Literal(super::witness::WitnessLiteral::Int(0)));
            }
            if interval.contains(1) {
                return Some(Witness::Literal(super::witness::WitnessLiteral::Int(1)));
            }
            // Use the start of the first interval
            return Some(Witness::Literal(super::witness::WitnessLiteral::Int(
                interval.start as i64,
            )));
        }
        None
    }
}

/// Check if patterns are exhaustive with refinement awareness
pub fn check_exhaustiveness_with_refinement(
    patterns: &[PatternColumn],
    scrutinee_ty: &Type,
    env: &TypeEnv,
) -> (bool, List<Witness>) {
    // Get refinement context if available
    let refinement_ctx = RefinementContext::from_type(scrutinee_ty);

    // Get constructors for the base type
    let base_ty = match scrutinee_ty {
        Type::Refined { base, .. } => base.as_ref(),
        _ => scrutinee_ty,
    };

    let all_ctors = get_type_constructors(base_ty, env);

    // Filter constructors based on refinement
    let possible_ctors: List<Constructor> = match &refinement_ctx {
        Some(ctx) => ctx.filter_constructors(&all_ctors),
        None => all_ctors.iter().cloned().collect(),
    };

    // Check if all possible constructors are covered
    let mut uncovered = List::new();

    for ctor in possible_ctors.iter() {
        let covered = patterns.iter().any(|p| pattern_covers_constructor(p, &ctor.name));
        if !covered {
            // Generate witness
            let witness = Witness::Constructor {
                name: ctor.name.clone(),
                args: List::new(),
            };
            uncovered.push(witness);
        }
    }

    // For numeric types with refinements, check literal coverage
    if let Some(ctx) = &refinement_ctx {
        if is_numeric_type(base_ty) {
            // Check if patterns cover all values in the refined range
            let covered_values = extract_covered_values(patterns);
            for interval in ctx.valid_intervals().iter() {
                // Check a few sample values
                for v in sample_interval(interval) {
                    if !value_is_covered(&covered_values, v) {
                        uncovered.push(Witness::Literal(super::witness::WitnessLiteral::Int(
                            v as i64,
                        )));
                        break;
                    }
                }
            }
        }
    }

    (uncovered.is_empty(), uncovered)
}

/// Check if a pattern covers a constructor
fn pattern_covers_constructor(pattern: &PatternColumn, name: &Text) -> bool {
    match pattern {
        PatternColumn::Wildcard => true,
        PatternColumn::Constructor { name: pname, .. } => pname == name,
        PatternColumn::Or(alts) => alts.iter().any(|a| pattern_covers_constructor(a, name)),
        PatternColumn::Guarded(inner) => pattern_covers_constructor(inner, name),
        _ => false,
    }
}

/// Check if a type is numeric
fn is_numeric_type(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Float)
}

/// Extract covered integer values from patterns
fn extract_covered_values(patterns: &[PatternColumn]) -> CoveredValues {
    let mut result = CoveredValues::new();

    for pattern in patterns {
        match pattern {
            PatternColumn::Wildcard => {
                result.covers_all = true;
            }
            PatternColumn::Literal(super::matrix::LiteralPattern::Int(n)) => {
                result.literals.push(*n as i128);
            }
            PatternColumn::Range { start, end, inclusive } => {
                let s = start.unwrap_or(i128::MIN);
                let e = if *inclusive {
                    end.unwrap_or(i128::MAX)
                } else {
                    end.map(|v| v - 1).unwrap_or(i128::MAX)
                };
                result.ranges.push((s, e));
            }
            PatternColumn::Or(alts) => {
                let inner = extract_covered_values(alts);
                result.merge(&inner);
            }
            _ => {}
        }
    }

    result
}

/// Representation of covered values
struct CoveredValues {
    covers_all: bool,
    literals: Vec<i128>,
    ranges: Vec<(i128, i128)>,
}

impl CoveredValues {
    fn new() -> Self {
        Self {
            covers_all: false,
            literals: Vec::new(),
            ranges: Vec::new(),
        }
    }

    fn merge(&mut self, other: &CoveredValues) {
        if other.covers_all {
            self.covers_all = true;
        }
        self.literals.extend(other.literals.iter());
        self.ranges.extend(other.ranges.iter());
    }
}

/// Check if a value is covered
fn value_is_covered(covered: &CoveredValues, value: i128) -> bool {
    if covered.covers_all {
        return true;
    }
    if covered.literals.contains(&value) {
        return true;
    }
    for (start, end) in &covered.ranges {
        if value >= *start && value <= *end {
            return true;
        }
    }
    false
}

/// Sample values from an interval for checking
fn sample_interval(interval: &Interval) -> Vec<i128> {
    let mut samples = Vec::new();

    // Always include boundaries
    samples.push(interval.start);
    samples.push(interval.end);

    // Include 0 if in range
    if interval.contains(0) {
        samples.push(0);
    }

    // Include midpoint for large intervals
    if interval.size() > 10 {
        let mid = (interval.start + interval.end) / 2;
        samples.push(mid);
    }

    // Limit to first few values for small intervals
    if interval.size() <= 10 {
        for i in interval.start..=interval.end {
            samples.push(i);
        }
    }

    samples
}

/// Intersect two interval sets
fn intersect_interval_sets(a: &IntervalSet, b: &IntervalSet) -> IntervalSet {
    let mut result = IntervalSet::empty();

    for interval_a in a.iter() {
        for interval_b in b.iter() {
            if let Some(intersection) = interval_a.intersect(interval_b) {
                result.add(intersection);
            }
        }
    }

    result
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
