//! Linear Logic — full propositional connectives + exponentials.
//!
//! Where QTT (`qtt_usage`) tracks resource usage by quantity
//! arithmetic, **linear logic** (Girard 1987) does it through
//! a richer connective algebra:
//!
//! ```text
//!     A, B ::= a              (atom)
//!            | A ⊗ B          (tensor:  both A and B, paired)
//!            | A ⅋ B          (par:     both A and B, parallel)
//!            | A & B          (with:    one of A, B by external choice)
//!            | A ⊕ B          (plus:    one of A, B by internal choice)
//!            | A ⊸ B          (lolli:   linear function A → B)
//!            | !A             (of-course / bang: A unrestricted)
//!            | ?A             (why-not: A weakenable & contractable)
//!            | 1, ⊥           (units of ⊗, ⅋)
//!            | ⊤, 0           (units of &, ⊕)
//!            | A^⊥            (linear negation / dual)
//! ```
//!
//! ## de Morgan dualities
//!
//! Linear negation is involutive (`(A^⊥)^⊥ = A`) and exchanges:
//!
//! ```text
//!     (A ⊗ B)^⊥ = A^⊥ ⅋ B^⊥
//!     (A & B)^⊥ = A^⊥ ⊕ B^⊥
//!     (!A)^⊥    = ?(A^⊥)
//!     1^⊥       = ⊥
//!     ⊤^⊥       = 0
//!     (A ⊸ B)   ≡ A^⊥ ⅋ B    (definitional)
//! ```
//!
//! ## Structural rules
//!
//! Linear logic restricts contraction (use a hypothesis twice)
//! and weakening (drop a hypothesis) to formulas under `!`. The
//! [`is_unrestricted`] and [`is_weakenable`] predicates classify
//! formulas accordingly.
//!
//! ## Status
//!
//! Standalone algebraic core. Integration with QTT is a future
//! step — `!A` corresponds to `Quantity::Omega`, `?A` to
//! `Quantity::AtMost(n)` for some n, plain atoms to
//! `Quantity::One`, and `0`/`⊥` to `Quantity::Zero`.

use verum_common::Text;

/// A linear logic formula.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LinForm {
    /// Atomic proposition.
    Atom(Text),
    /// `A ⊗ B` — multiplicative conjunction (tensor).
    Tensor(Box<LinForm>, Box<LinForm>),
    /// `A ⅋ B` — multiplicative disjunction (par).
    Par(Box<LinForm>, Box<LinForm>),
    /// `A & B` — additive conjunction (with).
    With(Box<LinForm>, Box<LinForm>),
    /// `A ⊕ B` — additive disjunction (plus).
    Plus(Box<LinForm>, Box<LinForm>),
    /// `!A` — exponential of-course.
    OfCourse(Box<LinForm>),
    /// `?A` — exponential why-not.
    WhyNot(Box<LinForm>),
    /// `1` — multiplicative truth (unit of ⊗).
    One,
    /// `⊥` — multiplicative false (unit of ⅋).
    Bottom,
    /// `⊤` — additive truth (unit of &).
    Top,
    /// `0` — additive false (unit of ⊕).
    Zero,
    /// `A^⊥` — linear negation (dual).
    Dual(Box<LinForm>),
}

impl LinForm {
    pub fn atom(name: impl Into<Text>) -> Self {
        Self::Atom(name.into())
    }

    pub fn tensor(a: LinForm, b: LinForm) -> Self {
        Self::Tensor(Box::new(a), Box::new(b))
    }

    pub fn par(a: LinForm, b: LinForm) -> Self {
        Self::Par(Box::new(a), Box::new(b))
    }

    pub fn with(a: LinForm, b: LinForm) -> Self {
        Self::With(Box::new(a), Box::new(b))
    }

    pub fn plus(a: LinForm, b: LinForm) -> Self {
        Self::Plus(Box::new(a), Box::new(b))
    }

    pub fn of_course(a: LinForm) -> Self {
        Self::OfCourse(Box::new(a))
    }

    pub fn why_not(a: LinForm) -> Self {
        Self::WhyNot(Box::new(a))
    }

    pub fn dual_of(a: LinForm) -> Self {
        Self::Dual(Box::new(a))
    }

    /// Linear implication `A ⊸ B` is defined as `A^⊥ ⅋ B`.
    pub fn lolli(a: LinForm, b: LinForm) -> Self {
        Self::par(Self::dual_of(a), b)
    }

    /// Compute the linear-negation normal form by pushing duals
    /// inward via the de Morgan rules until they appear only at
    /// atoms. This is the standard preprocessing step for
    /// proof-search and cut-elimination.
    pub fn negation_normal_form(&self) -> LinForm {
        self.nnf(false)
    }

    fn nnf(&self, negated: bool) -> LinForm {
        match self {
            LinForm::Atom(_) => {
                if negated {
                    LinForm::Dual(Box::new(self.clone()))
                } else {
                    self.clone()
                }
            }
            LinForm::Dual(inner) => inner.nnf(!negated),

            LinForm::Tensor(a, b) => {
                if negated {
                    LinForm::par(a.nnf(true), b.nnf(true))
                } else {
                    LinForm::tensor(a.nnf(false), b.nnf(false))
                }
            }
            LinForm::Par(a, b) => {
                if negated {
                    LinForm::tensor(a.nnf(true), b.nnf(true))
                } else {
                    LinForm::par(a.nnf(false), b.nnf(false))
                }
            }

            LinForm::With(a, b) => {
                if negated {
                    LinForm::plus(a.nnf(true), b.nnf(true))
                } else {
                    LinForm::with(a.nnf(false), b.nnf(false))
                }
            }
            LinForm::Plus(a, b) => {
                if negated {
                    LinForm::with(a.nnf(true), b.nnf(true))
                } else {
                    LinForm::plus(a.nnf(false), b.nnf(false))
                }
            }

            LinForm::OfCourse(a) => {
                if negated {
                    LinForm::why_not(a.nnf(true))
                } else {
                    LinForm::of_course(a.nnf(false))
                }
            }
            LinForm::WhyNot(a) => {
                if negated {
                    LinForm::of_course(a.nnf(true))
                } else {
                    LinForm::why_not(a.nnf(false))
                }
            }

            LinForm::One => {
                if negated {
                    LinForm::Bottom
                } else {
                    LinForm::One
                }
            }
            LinForm::Bottom => {
                if negated {
                    LinForm::One
                } else {
                    LinForm::Bottom
                }
            }
            LinForm::Top => {
                if negated {
                    LinForm::Zero
                } else {
                    LinForm::Top
                }
            }
            LinForm::Zero => {
                if negated {
                    LinForm::Top
                } else {
                    LinForm::Zero
                }
            }
        }
    }

    /// `!A` makes `A` unrestricted: it can be used any number of
    /// times. Returns true iff the outermost connective is `!` or
    /// the formula is a multiplicative truth.
    pub fn is_unrestricted(&self) -> bool {
        matches!(self, LinForm::OfCourse(_) | LinForm::Top | LinForm::One)
    }

    /// `?A` makes `A` weakenable: it can be discarded without use.
    /// Returns true iff the outermost connective is `?` or the
    /// formula is an additive false (which has weakening built in
    /// via the `Top` rule on the dual side).
    pub fn is_weakenable(&self) -> bool {
        matches!(self, LinForm::WhyNot(_) | LinForm::Top)
    }

    /// Two formulas are *dual* iff one is the linear negation of
    /// the other (after NNF). This is the central pairing rule
    /// for cut elimination.
    pub fn is_dual_of(&self, other: &LinForm) -> bool {
        let neg_self = LinForm::dual_of(self.clone()).negation_normal_form();
        let nnf_other = other.negation_normal_form();
        neg_self == nnf_other
    }
}

impl std::fmt::Display for LinForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinForm::Atom(n) => write!(f, "{}", n.as_str()),
            LinForm::Tensor(a, b) => write!(f, "({} ⊗ {})", a, b),
            LinForm::Par(a, b) => write!(f, "({} ⅋ {})", a, b),
            LinForm::With(a, b) => write!(f, "({} & {})", a, b),
            LinForm::Plus(a, b) => write!(f, "({} ⊕ {})", a, b),
            LinForm::OfCourse(a) => write!(f, "!{}", a),
            LinForm::WhyNot(a) => write!(f, "?{}", a),
            LinForm::One => write!(f, "1"),
            LinForm::Bottom => write!(f, "⊥"),
            LinForm::Top => write!(f, "⊤"),
            LinForm::Zero => write!(f, "0"),
            LinForm::Dual(a) => write!(f, "{}^⊥", a),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a() -> LinForm {
        LinForm::atom("A")
    }

    fn b() -> LinForm {
        LinForm::atom("B")
    }

    #[test]
    fn dual_is_involution_on_atoms() {
        let f = LinForm::dual_of(LinForm::dual_of(a())).negation_normal_form();
        assert_eq!(f, a());
    }

    #[test]
    fn dual_of_tensor_is_par_after_nnf() {
        // (A ⊗ B)^⊥  ↦  A^⊥ ⅋ B^⊥
        let lhs = LinForm::dual_of(LinForm::tensor(a(), b()));
        let nnf = lhs.negation_normal_form();
        assert_eq!(
            nnf,
            LinForm::par(LinForm::dual_of(a()), LinForm::dual_of(b()))
        );
    }

    #[test]
    fn dual_of_with_is_plus_after_nnf() {
        let lhs = LinForm::dual_of(LinForm::with(a(), b()));
        let nnf = lhs.negation_normal_form();
        assert_eq!(
            nnf,
            LinForm::plus(LinForm::dual_of(a()), LinForm::dual_of(b()))
        );
    }

    #[test]
    fn dual_of_of_course_is_why_not_after_nnf() {
        let lhs = LinForm::dual_of(LinForm::of_course(a()));
        let nnf = lhs.negation_normal_form();
        assert_eq!(nnf, LinForm::why_not(LinForm::dual_of(a())));
    }

    #[test]
    fn dual_of_one_is_bottom() {
        let lhs = LinForm::dual_of(LinForm::One);
        assert_eq!(lhs.negation_normal_form(), LinForm::Bottom);
    }

    #[test]
    fn dual_of_top_is_zero() {
        let lhs = LinForm::dual_of(LinForm::Top);
        assert_eq!(lhs.negation_normal_form(), LinForm::Zero);
    }

    #[test]
    fn lolli_definition_unfolds_to_par_of_dual() {
        // A ⊸ B  ≡  A^⊥ ⅋ B
        let lolli = LinForm::lolli(a(), b());
        assert_eq!(lolli, LinForm::par(LinForm::dual_of(a()), b()));
    }

    #[test]
    fn of_course_is_unrestricted() {
        assert!(LinForm::of_course(a()).is_unrestricted());
        assert!(!a().is_unrestricted());
    }

    #[test]
    fn why_not_is_weakenable() {
        assert!(LinForm::why_not(a()).is_weakenable());
        assert!(!a().is_weakenable());
        // Top is weakenable too (additive nature).
        assert!(LinForm::Top.is_weakenable());
    }

    #[test]
    fn atom_is_dual_of_its_negation() {
        // a is dual of a^⊥
        let pos = a();
        let neg = LinForm::dual_of(a());
        assert!(pos.is_dual_of(&neg));
    }

    #[test]
    fn tensor_is_dual_of_par_with_dual_components() {
        let lhs = LinForm::tensor(a(), b());
        let rhs = LinForm::par(LinForm::dual_of(a()), LinForm::dual_of(b()));
        assert!(lhs.is_dual_of(&rhs));
    }

    #[test]
    fn nested_dual_pushes_inward() {
        // ((A ⊗ B)^⊥)^⊥  ↦  A ⊗ B   (involution)
        let inner = LinForm::tensor(a(), b());
        let f = LinForm::dual_of(LinForm::dual_of(inner.clone()));
        assert_eq!(f.negation_normal_form(), inner);
    }

    #[test]
    fn distinct_atoms_not_dual() {
        assert!(!a().is_dual_of(&b()));
    }

    #[test]
    fn one_and_top_are_unrestricted() {
        assert!(LinForm::One.is_unrestricted());
        assert!(LinForm::Top.is_unrestricted());
        assert!(!LinForm::Zero.is_unrestricted());
        assert!(!LinForm::Bottom.is_unrestricted());
    }

    #[test]
    fn display_shows_unicode_connectives() {
        let f = LinForm::tensor(a(), b());
        assert!(format!("{}", f).contains("⊗"));
        let g = LinForm::par(a(), b());
        assert!(format!("{}", g).contains("⅋"));
        let h = LinForm::of_course(a());
        assert_eq!(format!("{}", h), "!A");
    }

    #[test]
    fn deeply_nested_de_morgan_normalises() {
        // (! (A ⊗ B))^⊥  ↦  ?(A^⊥ ⅋ B^⊥)
        let f = LinForm::dual_of(LinForm::of_course(LinForm::tensor(a(), b())));
        let nnf = f.negation_normal_form();
        assert_eq!(
            nnf,
            LinForm::why_not(LinForm::par(
                LinForm::dual_of(a()),
                LinForm::dual_of(b())
            ))
        );
    }
}
