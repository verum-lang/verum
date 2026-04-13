//! Observational Type Theory (OTT) — alternative to cubical.
//!
//! Where cubical type theory makes equality computational by
//! reducing `transport` and `hcomp`, **observational** type
//! theory takes a different route: equality is *defined by
//! observation*. Two values of a Π-type are equal iff they yield
//! equal results on every input. Two values of a Σ-type are
//! equal iff their components are equal pairwise. Two values of
//! a coinductive type are equal iff every observation yields
//! equal results (bisimilarity).
//!
//! This is type-directed — the equality relation depends on the
//! *type* of the values being compared. The result is a system
//! that retains UIP (uniqueness of identity proofs) while still
//! supporting funext and propositional irrelevance, all without
//! the cubical machinery.
//!
//! Verum's main equality story is cubical (with the EqTerm↔
//! CubicalTerm bridge). This module provides OTT as a
//! *complementary* alternative — useful for fragments where
//! strict computability is desired without the cubical overhead,
//! and for comparison/research purposes.
//!
//! ## Core rules
//!
//! ```text
//!     Eq Bool b₁ b₂          ↦  b₁ ≡ b₂                   (decidable)
//!     Eq Nat n₁ n₂           ↦  n₁ ≡ n₂                   (decidable)
//!     Eq (A × B) (a, b) (c, d)  ↦  Eq A a c × Eq B b d   (component-wise)
//!     Eq (A → B) f g         ↦  ∀x:A. Eq B (f x) (g x)    (funext)
//!     Eq (Σ x:A. P x) (a, p) (b, q)  ↦  Eq A a b × Eq P[a/x] p q
//! ```
//!
//! ## Status
//!
//! Standalone algebraic core. No coupling to the cubical
//! normalizer or the type checker — intended for opt-in use by
//! fragments that prefer OTT semantics, and as a reference
//! implementation for research.

use verum_common::{List, Text};

/// A simplified type for OTT computations. Sufficient to express
/// the canonical equality reductions; richer dependent shapes
/// (full Π, full Σ with binders) are represented opaquely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OttType {
    /// `Bool` — decidable equality on values.
    Bool,
    /// `Nat` — decidable equality on naturals.
    Nat,
    /// `Unit` — only one inhabitant; equality is trivial.
    Unit,
    /// Product `A × B`.
    Product(Box<OttType>, Box<OttType>),
    /// Function space `A → B`. Equality is funext over A.
    Function(Box<OttType>, Box<OttType>),
    /// Dependent pair `Σ x:A. B(x)`.
    /// We treat the second component opaquely since OTT collapses
    /// dependent equality to component equality up to the
    /// substitution at the witness.
    Sigma(Box<OttType>, Box<OttType>),
    /// Opaque type — equality remains stuck.
    Opaque(Text),
}

impl OttType {
    pub fn product(a: OttType, b: OttType) -> Self {
        Self::Product(Box::new(a), Box::new(b))
    }

    pub fn function(a: OttType, b: OttType) -> Self {
        Self::Function(Box::new(a), Box::new(b))
    }

    pub fn sigma(a: OttType, b: OttType) -> Self {
        Self::Sigma(Box::new(a), Box::new(b))
    }

    pub fn opaque(name: impl Into<Text>) -> Self {
        Self::Opaque(name.into())
    }
}

/// A value at an OTT type. Sufficient for the reduction rules; we
/// store enough structure to inspect canonical forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OttValue {
    /// Boolean literal.
    BoolLit(bool),
    /// Natural-number literal.
    NatLit(u64),
    /// The unique inhabitant of `Unit`.
    UnitLit,
    /// Pair `(a, b)`.
    Pair(Box<OttValue>, Box<OttValue>),
    /// A function value, represented opaquely by a name. Funext
    /// rules require comparing applications, captured here as a
    /// list of input/output samples.
    Function {
        name: Text,
        samples: List<(OttValue, OttValue)>,
    },
    /// Opaque value — equality is syntactic.
    Opaque(Text),
}

impl OttValue {
    pub fn pair(a: OttValue, b: OttValue) -> Self {
        Self::Pair(Box::new(a), Box::new(b))
    }

    pub fn function(name: impl Into<Text>, samples: impl IntoIterator<Item = (OttValue, OttValue)>) -> Self {
        Self::Function {
            name: name.into(),
            samples: samples.into_iter().collect(),
        }
    }

    pub fn opaque(name: impl Into<Text>) -> Self {
        Self::Opaque(name.into())
    }
}

/// Result of OTT equality reduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EqResult {
    /// Equality decided to true.
    True,
    /// Equality decided to false.
    False,
    /// Equality reduced to a list of subgoal equalities at simpler
    /// types (for products / functions / sigmas).
    Subgoals(List<EqGoal>),
    /// Equality is stuck — value or type involves opaque pieces.
    Stuck,
}

/// A subgoal equality: two values at a given type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqGoal {
    pub ty: OttType,
    pub lhs: OttValue,
    pub rhs: OttValue,
}

impl EqGoal {
    pub fn new(ty: OttType, lhs: OttValue, rhs: OttValue) -> Self {
        Self { ty, lhs, rhs }
    }
}

/// The central type-directed equality reducer. Applies the OTT
/// rules:
///
/// * `Bool / Nat / Unit` decide directly.
/// * `Product` decomposes into pair-of-equalities.
/// * `Function` decomposes into pointwise equalities at the
///   sampled inputs (a finitary approximation of funext).
/// * `Sigma` decomposes into first-component equality plus an
///   opaque second-component subgoal.
/// * Anything else is `Stuck`.
pub fn equal_at(ty: &OttType, lhs: &OttValue, rhs: &OttValue) -> EqResult {
    match (ty, lhs, rhs) {
        (OttType::Bool, OttValue::BoolLit(a), OttValue::BoolLit(b)) => {
            if a == b {
                EqResult::True
            } else {
                EqResult::False
            }
        }
        (OttType::Nat, OttValue::NatLit(a), OttValue::NatLit(b)) => {
            if a == b {
                EqResult::True
            } else {
                EqResult::False
            }
        }
        (OttType::Unit, OttValue::UnitLit, OttValue::UnitLit) => EqResult::True,

        // Product: (a, b) = (c, d)  ↦  a = c × b = d
        (OttType::Product(ta, tb), OttValue::Pair(a, b), OttValue::Pair(c, d)) => {
            let mut goals = List::new();
            goals.push(EqGoal::new((**ta).clone(), (**a).clone(), (**c).clone()));
            goals.push(EqGoal::new((**tb).clone(), (**b).clone(), (**d).clone()));
            EqResult::Subgoals(goals)
        }

        // Function: f = g  ↦  ∀ sampled inputs, f(x) = g(x)
        (
            OttType::Function(_, ret_ty),
            OttValue::Function {
                samples: l_samples,
                ..
            },
            OttValue::Function {
                samples: r_samples,
                ..
            },
        ) => {
            // Funext via finite sampling. Two functions are equal
            // iff every sampled input yields the same output.
            let mut goals = List::new();
            for ((l_in, l_out), (r_in, r_out)) in
                l_samples.iter().zip(r_samples.iter())
            {
                if l_in != r_in {
                    return EqResult::Stuck;
                }
                goals.push(EqGoal::new(
                    (**ret_ty).clone(),
                    l_out.clone(),
                    r_out.clone(),
                ));
            }
            if l_samples.len() != r_samples.len() {
                return EqResult::Stuck;
            }
            EqResult::Subgoals(goals)
        }

        // Sigma: (a, p) = (b, q) ↦ a = b × p = q (at substituted type)
        (OttType::Sigma(ta, tb), OttValue::Pair(a, p), OttValue::Pair(b, q)) => {
            let mut goals = List::new();
            goals.push(EqGoal::new((**ta).clone(), (**a).clone(), (**b).clone()));
            goals.push(EqGoal::new((**tb).clone(), (**p).clone(), (**q).clone()));
            EqResult::Subgoals(goals)
        }

        // Opaque type or shape mismatch — stuck or false.
        (OttType::Opaque(_), a, b) => {
            if a == b {
                EqResult::True
            } else {
                EqResult::Stuck
            }
        }
        _ => EqResult::Stuck,
    }
}

/// Recursive evaluation: keep reducing until all subgoals are True
/// or any subgoal becomes False/Stuck. Returns:
///
/// * `EqResult::True` iff every subgoal eventually decides true
/// * `EqResult::False` iff any subgoal decides false
/// * `EqResult::Stuck` iff any subgoal stays stuck
pub fn decide(ty: &OttType, lhs: &OttValue, rhs: &OttValue) -> EqResult {
    let mut frontier = vec![EqGoal::new(ty.clone(), lhs.clone(), rhs.clone())];
    while let Some(g) = frontier.pop() {
        match equal_at(&g.ty, &g.lhs, &g.rhs) {
            EqResult::True => continue,
            EqResult::False => return EqResult::False,
            EqResult::Stuck => return EqResult::Stuck,
            EqResult::Subgoals(subs) => {
                for s in subs.iter() {
                    frontier.push(s.clone());
                }
            }
        }
    }
    EqResult::True
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_true_equals_true() {
        assert_eq!(
            equal_at(&OttType::Bool, &OttValue::BoolLit(true), &OttValue::BoolLit(true)),
            EqResult::True
        );
    }

    #[test]
    fn bool_true_does_not_equal_false() {
        assert_eq!(
            equal_at(&OttType::Bool, &OttValue::BoolLit(true), &OttValue::BoolLit(false)),
            EqResult::False
        );
    }

    #[test]
    fn nat_decidable_equality() {
        assert_eq!(
            equal_at(&OttType::Nat, &OttValue::NatLit(7), &OttValue::NatLit(7)),
            EqResult::True
        );
        assert_eq!(
            equal_at(&OttType::Nat, &OttValue::NatLit(7), &OttValue::NatLit(8)),
            EqResult::False
        );
    }

    #[test]
    fn unit_always_equal() {
        assert_eq!(
            equal_at(&OttType::Unit, &OttValue::UnitLit, &OttValue::UnitLit),
            EqResult::True
        );
    }

    #[test]
    fn product_decomposes_into_components() {
        let ty = OttType::product(OttType::Bool, OttType::Nat);
        let lhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(5));
        let rhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(5));
        match equal_at(&ty, &lhs, &rhs) {
            EqResult::Subgoals(goals) => {
                assert_eq!(goals.len(), 2);
                // First subgoal is at Bool, second at Nat.
                assert_eq!(goals[0].ty, OttType::Bool);
                assert_eq!(goals[1].ty, OttType::Nat);
            }
            _ => panic!("expected subgoals"),
        }
    }

    #[test]
    fn product_decide_true_when_components_match() {
        let ty = OttType::product(OttType::Bool, OttType::Nat);
        let lhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(5));
        let rhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(5));
        assert_eq!(decide(&ty, &lhs, &rhs), EqResult::True);
    }

    #[test]
    fn product_decide_false_when_one_component_differs() {
        let ty = OttType::product(OttType::Bool, OttType::Nat);
        let lhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(5));
        let rhs = OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(6));
        assert_eq!(decide(&ty, &lhs, &rhs), EqResult::False);
    }

    #[test]
    fn function_funext_via_samples() {
        let ty = OttType::function(OttType::Nat, OttType::Bool);
        let f = OttValue::function(
            "f",
            [
                (OttValue::NatLit(0), OttValue::BoolLit(true)),
                (OttValue::NatLit(1), OttValue::BoolLit(false)),
            ],
        );
        let g = OttValue::function(
            "g",
            [
                (OttValue::NatLit(0), OttValue::BoolLit(true)),
                (OttValue::NatLit(1), OttValue::BoolLit(false)),
            ],
        );
        // Same outputs at same inputs → equal under funext.
        assert_eq!(decide(&ty, &f, &g), EqResult::True);
    }

    #[test]
    fn function_funext_disagrees_on_one_input() {
        let ty = OttType::function(OttType::Nat, OttType::Bool);
        let f = OttValue::function(
            "f",
            [(OttValue::NatLit(0), OttValue::BoolLit(true))],
        );
        let g = OttValue::function(
            "g",
            [(OttValue::NatLit(0), OttValue::BoolLit(false))],
        );
        assert_eq!(decide(&ty, &f, &g), EqResult::False);
    }

    #[test]
    fn function_with_different_inputs_is_stuck() {
        let ty = OttType::function(OttType::Nat, OttType::Bool);
        let f = OttValue::function(
            "f",
            [(OttValue::NatLit(0), OttValue::BoolLit(true))],
        );
        let g = OttValue::function(
            "g",
            [(OttValue::NatLit(99), OttValue::BoolLit(true))],
        );
        // Different sample sets — finitary funext can't decide.
        assert_eq!(decide(&ty, &f, &g), EqResult::Stuck);
    }

    #[test]
    fn function_with_different_sample_lengths_is_stuck() {
        let ty = OttType::function(OttType::Nat, OttType::Bool);
        let f = OttValue::function(
            "f",
            [(OttValue::NatLit(0), OttValue::BoolLit(true))],
        );
        let g = OttValue::function(
            "g",
            [
                (OttValue::NatLit(0), OttValue::BoolLit(true)),
                (OttValue::NatLit(1), OttValue::BoolLit(true)),
            ],
        );
        assert_eq!(decide(&ty, &f, &g), EqResult::Stuck);
    }

    #[test]
    fn sigma_decomposes_into_first_and_second() {
        let ty = OttType::sigma(OttType::Nat, OttType::Bool);
        let lhs = OttValue::pair(OttValue::NatLit(5), OttValue::BoolLit(true));
        let rhs = OttValue::pair(OttValue::NatLit(5), OttValue::BoolLit(true));
        assert_eq!(decide(&ty, &lhs, &rhs), EqResult::True);
    }

    #[test]
    fn opaque_equal_when_syntactically_identical() {
        let ty = OttType::opaque("MyType");
        let v = OttValue::opaque("v");
        assert_eq!(decide(&ty, &v, &v), EqResult::True);
    }

    #[test]
    fn opaque_stuck_when_different() {
        let ty = OttType::opaque("MyType");
        let a = OttValue::opaque("a");
        let b = OttValue::opaque("b");
        assert_eq!(decide(&ty, &a, &b), EqResult::Stuck);
    }

    #[test]
    fn nested_product_recursively_decided() {
        let ty = OttType::product(
            OttType::product(OttType::Bool, OttType::Nat),
            OttType::Unit,
        );
        let lhs = OttValue::pair(
            OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(3)),
            OttValue::UnitLit,
        );
        let rhs = lhs.clone();
        assert_eq!(decide(&ty, &lhs, &rhs), EqResult::True);
    }

    #[test]
    fn nested_product_one_deep_difference_yields_false() {
        let ty = OttType::product(
            OttType::product(OttType::Bool, OttType::Nat),
            OttType::Unit,
        );
        let lhs = OttValue::pair(
            OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(3)),
            OttValue::UnitLit,
        );
        let rhs = OttValue::pair(
            OttValue::pair(OttValue::BoolLit(true), OttValue::NatLit(4)),
            OttValue::UnitLit,
        );
        assert_eq!(decide(&ty, &lhs, &rhs), EqResult::False);
    }
}
