//! Decidable ordinal notation for finite sums of ω², ω and finite terms,
//! finite powers of ω, symbolic inaccessible cardinals and their successors.
//! A finite list has a maximum. An arbitrary transfinite family needs its own
//! index, transition data and supremum proof.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Ordinal notation with finite syntax and symbolic inaccessible cardinals.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ordinal {
    /// A finite ordinal `n`. `Finite(0)` is the smallest ordinal,
    /// `Finite(1)` is its successor, etc.
    Finite(u32),

    /// The first transfinite ordinal `ω`.
    Omega,

    /// `ω + n` for `n ≥ 1`. Convention: `OmegaPlus(1)` is `ω + 1`,
    /// `OmegaPlus(2)` is `ω + 2`, etc. `OmegaPlus(0)` is normalised
    /// to plain `Omega`.
    OmegaPlus(u32),

    /// `ω·k` for `k ≥ 2`. `OmegaTimes(2)` is `ω·2`, `OmegaTimes(3)`
    /// is `ω·3`, etc. `OmegaTimes(1)` is normalised to `Omega`.
    OmegaTimes(u32),

    /// `ω·k + n` for `k ≥ 1`, `n ≥ 1`. Supports values like `ω·3 + 5`.
    /// Lower-bound-normalisation: when `k == 1` we use `OmegaPlus(n)`
    /// and when `n == 0` we collapse to `OmegaTimes(k)`.
    OmegaTimesPlus {
        /// Coefficient on ω: `ω·k + …`.
        k: u32,
        /// Finite tail: `… + n`.
        n: u32,
    },

    /// `ω²` — the second-order limit.
    OmegaSquared,

    /// Left ordinal sum `ω² + α`. A larger leading term in α absorbs ω².
    OmegaSquaredPlus(Box<Ordinal>),

    /// `ω^e` for `e ≥ 3`. Covers ω³, ω⁴, ...
    OmegaPow(u32),

    /// Symbolic inaccessible cardinal κ_n, indexed from zero.
    Kappa(u32),

    /// Supremum of the supplied finite family (zero for the empty family).
    Sup(Vec<Ordinal>),
    /// Successor when the compact constructors cannot encode the finite tail.
    Successor(Box<Ordinal>),
    /// A universe step that exceeds the compact κ index.
    NextInaccessible(Box<Ordinal>),
}

/// Principal terms larger than every finite sum of powers at most ω².
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Principal {
    Power(u32),
    Inaccessible(u64),
}

/// Coefficients in descending exponent order. In this syntax a higher
/// principal can only be followed by a finite successor tail. Each extra
/// unit beyond a u32 constructor requires a distinct allocated syntax node,
/// so the counts fit in u64 on supported machines.
#[derive(Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct NormalForm {
    principal: Option<Principal>,
    squares: u64,
    omegas: u32,
    finite: u64,
}

impl NormalForm {
    fn above_universe(self) -> Self {
        let index = match self.principal {
            Some(Principal::Inaccessible(n)) => n + 1,
            _ => 0,
        };
        Self {
            principal: Some(Principal::Inaccessible(index)),
            ..Self::default()
        }
    }

    fn notation(self) -> Ordinal {
        use Ordinal::*;
        let mut tail = self.finite;
        let mut value = if let Some(principal) = self.principal {
            match principal {
                Principal::Power(e) => OmegaPow(e),
                Principal::Inaccessible(n) => {
                    let packed = n.min(u64::from(u32::MAX));
                    let mut base = Kappa(packed as u32);
                    for _ in packed..n {
                        base = NextInaccessible(Box::new(base));
                    }
                    base
                }
            }
        } else {
            let packed = tail.min(u64::from(u32::MAX)) as u32;
            tail -= u64::from(packed);
            match (self.omegas, packed) {
                (0, n) => Finite(n),
                (k, 0) => Ordinal::omega_times(k),
                (1, n) => OmegaPlus(n),
                (k, n) => OmegaTimesPlus { k, n },
            }
        };
        for _ in 0..tail {
            value = Successor(Box::new(value));
        }
        for _ in 0..self.squares {
            value = if matches!(value, Finite(0)) {
                OmegaSquared
            } else {
                OmegaSquaredPlus(Box::new(value))
            };
        }
        value
    }
}

impl Ordinal {
    fn normal_form(&self) -> NormalForm {
        use Ordinal::*;
        enum Work<'a> {
            Visit(&'a Ordinal),
            PrefixSquare,
            Succ,
            Ascend,
            Supremum(usize),
        }

        let mut first = Some(Work::Visit(self));
        let mut work = Vec::new();
        let mut values = Vec::<NormalForm>::new();
        while let Some(step) = first.take().or_else(|| work.pop()) {
            let mut value = NormalForm::default();
            match step {
                Work::Visit(Finite(n)) => value.finite = u64::from(*n),
                Work::Visit(Omega | OmegaPow(1)) => value.omegas = 1,
                Work::Visit(OmegaPlus(n)) => {
                    value.omegas = 1;
                    value.finite = u64::from(*n);
                }
                Work::Visit(OmegaTimes(k)) => value.omegas = *k,
                Work::Visit(OmegaTimesPlus { k, n }) => {
                    value.omegas = *k;
                    value.finite = u64::from(*n);
                }
                Work::Visit(OmegaSquared | OmegaPow(2)) => value.squares = 1,
                Work::Visit(OmegaPow(0)) => value.finite = 1,
                Work::Visit(OmegaPow(e)) => value.principal = Some(Principal::Power(*e)),
                Work::Visit(Kappa(k)) => {
                    value.principal = Some(Principal::Inaccessible(u64::from(*k)))
                }
                Work::Visit(OmegaSquaredPlus(inner)) => {
                    work.extend([Work::PrefixSquare, Work::Visit(inner)]);
                    continue;
                }
                Work::Visit(Successor(inner)) => {
                    work.extend([Work::Succ, Work::Visit(inner)]);
                    continue;
                }
                Work::Visit(NextInaccessible(inner)) => {
                    work.extend([Work::Ascend, Work::Visit(inner)]);
                    continue;
                }
                Work::Visit(Sup(parts)) => {
                    work.push(Work::Supremum(parts.len()));
                    work.extend(parts.iter().map(Work::Visit));
                    continue;
                }
                Work::PrefixSquare => {
                    let inner = values.last_mut().expect("ordinal tail");
                    // Ordinal addition on the left is absorbed by any larger
                    // leading principal on the right.
                    if inner.principal.is_none() {
                        inner.squares += 1;
                    }
                    continue;
                }
                Work::Succ => {
                    values.last_mut().expect("successor operand").finite += 1;
                    continue;
                }
                Work::Ascend => {
                    value = values.pop().expect("universe operand").above_universe();
                }
                Work::Supremum(count) => {
                    value = values
                        .drain(values.len() - count..)
                        .max()
                        .unwrap_or_default();
                }
            }
            if work.is_empty() {
                return value;
            }
            values.push(value);
        }
        values.pop().expect("an ordinal has a normal form")
    }

    /// Ordinal succession, including at the boundaries of compact notations.
    pub fn succ(&self) -> Self {
        let mut value = self.normal_form();
        value.finite += 1;
        value.notation()
    }

    /// Strict order on the ordinals denoted by the notations.
    pub fn lt(&self, other: &Self) -> bool {
        self.normal_form() < other.normal_form()
    }

    /// Compare any two notations, including noncanonical input.
    pub fn lt_raw(&self, other: &Self) -> bool {
        self.lt(other)
    }

    /// Nonstrict semantic order. Rust equality on the enum compares syntax.
    pub fn le(&self, other: &Self) -> bool {
        self.normal_form() <= other.normal_form()
    }

    /// A nonzero ordinal with no immediate predecessor.
    pub fn is_limit(&self) -> bool {
        let value = self.normal_form();
        value.finite == 0 && value != NormalForm::default()
    }

    /// Nonzero ordinals in this fragment equal to their cofinality. A
    /// successor has cofinality one. Inaccessibles are symbolic assumptions.
    pub fn is_regular(&self) -> bool {
        let value = self.normal_form();
        if value.squares != 0 {
            return false;
        }
        match (value.principal, value.omegas, value.finite) {
            (None, 0, 1) | (None, 1, 0) | (Some(Principal::Inaccessible(_)), 0, 0) => true,
            _ => false,
        }
    }

    /// Whether the normal form denotes a member of the symbolic κ tower.
    pub fn is_inaccessible(&self) -> bool {
        let value = self.normal_form();
        matches!(value.principal, Some(Principal::Inaccessible(_))) && value.finite == 0
    }

    /// Least tower symbol strictly above the represented ordinal.
    pub fn next_inaccessible(&self) -> Self {
        self.normal_form().above_universe().notation()
    }

    /// Display the semantic normal form in descending exponent order.
    pub fn render(&self) -> String {
        let value = self.normal_form();
        let mut parts = Vec::new();
        match value.principal {
            Some(Principal::Power(e)) => parts.push(format!("ω^{e}")),
            Some(Principal::Inaccessible(k)) => parts.push(format!("κ_{k}")),
            None => {}
        }
        for (symbol, count) in [("ω²", value.squares), ("ω", u64::from(value.omegas))] {
            match count {
                0 => {}
                1 => parts.push(symbol.to_owned()),
                n => parts.push(format!("{symbol}·{n}")),
            }
        }
        if value.finite != 0 || parts.is_empty() {
            parts.push(value.finite.to_string());
        }
        parts.join(" + ")
    }

    /// Construct ω + k in compact notation.
    pub fn omega_plus(k: u32) -> Self {
        match k {
            0 => Self::Omega,
            n => Self::OmegaPlus(n),
        }
    }

    /// Construct ω·k in compact notation.
    pub fn omega_times(k: u32) -> Self {
        match k {
            0 => Self::Finite(0),
            1 => Self::Omega,
            n => Self::OmegaTimes(n),
        }
    }

    /// Construct the indexed tower symbol. κ_0 is its first member.
    pub fn kappa(n: u32) -> Self {
        Self::Kappa(n)
    }

    /// Canonical notation, identifying aliases and all finite suprema.
    pub fn normalize(&self) -> Self {
        self.normal_form().notation()
    }
}

impl fmt::Display for Ordinal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render())
    }
}

impl Default for Ordinal {
    fn default() -> Self {
        Ordinal::Finite(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_ordering() {
        assert!(Ordinal::Finite(0).lt(&Ordinal::Finite(1)));
        assert!(Ordinal::Finite(5).lt(&Ordinal::Finite(7)));
        assert!(!Ordinal::Finite(7).lt(&Ordinal::Finite(5)));
        assert!(Ordinal::Finite(0).lt(&Ordinal::Omega));
        assert!(Ordinal::Finite(1_000_000).lt(&Ordinal::Omega));
    }

    #[test]
    fn omega_ordering() {
        assert!(Ordinal::Omega.lt(&Ordinal::OmegaPlus(1)));
        assert!(Ordinal::OmegaPlus(1).lt(&Ordinal::OmegaPlus(2)));
        assert!(Ordinal::OmegaPlus(99).lt(&Ordinal::OmegaTimes(2)));
        assert!(Ordinal::OmegaTimes(2).lt(&Ordinal::OmegaTimes(3)));
        assert!(Ordinal::OmegaTimes(99).lt(&Ordinal::OmegaSquared));
        assert!(Ordinal::OmegaSquared.lt(&Ordinal::OmegaPow(3)));
    }

    #[test]
    fn kappa_above_everything() {
        // Every Cantor-normal-form ordinal is below κ_1.
        assert!(Ordinal::Finite(99).lt(&Ordinal::Kappa(1)));
        assert!(Ordinal::Omega.lt(&Ordinal::Kappa(1)));
        assert!(Ordinal::OmegaSquared.lt(&Ordinal::Kappa(1)));
        assert!(Ordinal::OmegaPow(7).lt(&Ordinal::Kappa(1)));
        // κ_1 < κ_2 < κ_3 ...
        assert!(Ordinal::Kappa(1).lt(&Ordinal::Kappa(2)));
        assert!(Ordinal::Kappa(7).lt(&Ordinal::Kappa(99)));
    }

    #[test]
    fn succ_finite() {
        assert_eq!(Ordinal::Finite(0).succ(), Ordinal::Finite(1));
        assert_eq!(Ordinal::Finite(99).succ(), Ordinal::Finite(100));
    }

    #[test]
    fn succ_transfinite() {
        assert_eq!(Ordinal::Omega.succ(), Ordinal::OmegaPlus(1));
        assert_eq!(Ordinal::OmegaPlus(2).succ(), Ordinal::OmegaPlus(3));
        assert_eq!(
            Ordinal::OmegaTimes(3).succ(),
            Ordinal::OmegaTimesPlus { k: 3, n: 1 }
        );
    }

    #[test]
    fn next_inaccessible_finite_ascends_to_kappa_0() {
        assert_eq!(Ordinal::Finite(0).next_inaccessible(), Ordinal::Kappa(0));
        assert_eq!(Ordinal::Finite(99).next_inaccessible(), Ordinal::Kappa(0));
    }

    #[test]
    fn next_inaccessible_omega_ascends_to_kappa_0() {
        assert_eq!(Ordinal::Omega.next_inaccessible(), Ordinal::Kappa(0));
        assert_eq!(Ordinal::OmegaSquared.next_inaccessible(), Ordinal::Kappa(0));
        assert_eq!(Ordinal::OmegaPow(7).next_inaccessible(), Ordinal::Kappa(0));
    }

    #[test]
    fn next_inaccessible_climbs_kappa_tower() {
        assert_eq!(Ordinal::Kappa(0).next_inaccessible(), Ordinal::Kappa(1));
        assert_eq!(Ordinal::Kappa(1).next_inaccessible(), Ordinal::Kappa(2));
        assert_eq!(Ordinal::Kappa(7).next_inaccessible(), Ordinal::Kappa(8));
    }

    #[test]
    fn next_inaccessible_strictly_ascends_at_the_packed_boundary() {
        let top = Ordinal::Kappa(u32::MAX);
        let next = top.next_inaccessible();
        assert!(top.lt(&next));
        assert!(next.is_inaccessible());
        assert_eq!(next.render(), "κ_4294967296");
    }

    #[test]
    fn next_inaccessible_distinct_from_succ() {
        let k = Ordinal::Kappa(3);
        assert_ne!(k.succ(), k.next_inaccessible());
        assert_eq!(k.next_inaccessible(), Ordinal::Kappa(4));
    }

    #[test]
    fn next_inaccessible_sup_picks_largest_kappa() {
        let s = Ordinal::Sup(vec![
            Ordinal::Kappa(2),
            Ordinal::Kappa(5),
            Ordinal::Kappa(3),
        ]);
        assert_eq!(s.next_inaccessible(), Ordinal::Kappa(6));
    }

    #[test]
    fn next_inaccessible_sup_below_kappa_ascends_to_kappa_0() {
        let s = Ordinal::Sup(vec![Ordinal::Omega, Ordinal::OmegaSquared]);
        assert_eq!(s.next_inaccessible(), Ordinal::Kappa(0));
    }

    #[test]
    fn is_limit_classification() {
        assert!(!Ordinal::Finite(0).is_limit());
        assert!(!Ordinal::Finite(99).is_limit());
        assert!(Ordinal::Omega.is_limit());
        assert!(!Ordinal::OmegaPlus(1).is_limit());
        assert!(Ordinal::OmegaTimes(2).is_limit());
        assert!(!Ordinal::OmegaTimesPlus { k: 2, n: 5 }.is_limit());
        assert!(Ordinal::OmegaSquared.is_limit());
        assert!(Ordinal::OmegaPow(3).is_limit());
        assert!(Ordinal::Kappa(1).is_limit());
    }

    #[test]
    fn is_regular_classification() {
        // Every nonzero successor has cofinality one.
        assert!(Ordinal::Finite(1).is_regular());
        assert!(!Ordinal::Finite(0).is_regular()); // 0 is not regular by convention
        assert!(!Ordinal::OmegaPlus(1).is_regular());
        // ω: regular.
        assert!(Ordinal::Omega.is_regular());
        // Inaccessibles: regular.
        assert!(Ordinal::Kappa(1).is_regular());
        assert!(Ordinal::Kappa(2).is_regular());
        // Singular limits below κ_1: not regular.
        assert!(!Ordinal::OmegaTimes(2).is_regular());
        assert!(!Ordinal::OmegaSquared.is_regular());
        assert!(!Ordinal::OmegaPow(3).is_regular());
    }

    #[test]
    fn is_inaccessible_only_kappa() {
        assert!(!Ordinal::Finite(0).is_inaccessible());
        assert!(!Ordinal::Omega.is_inaccessible());
        assert!(!Ordinal::OmegaSquared.is_inaccessible());
        assert!(Ordinal::Kappa(1).is_inaccessible());
        assert!(Ordinal::Kappa(99).is_inaccessible());
    }

    #[test]
    fn render_canonical() {
        assert_eq!(Ordinal::Finite(0).render(), "0");
        assert_eq!(Ordinal::Finite(7).render(), "7");
        assert_eq!(Ordinal::Omega.render(), "ω");
        assert_eq!(Ordinal::OmegaPlus(1).render(), "ω + 1");
        assert_eq!(Ordinal::OmegaTimes(2).render(), "ω·2");
        assert_eq!(Ordinal::OmegaTimesPlus { k: 3, n: 5 }.render(), "ω·3 + 5");
        assert_eq!(Ordinal::OmegaSquared.render(), "ω²");
        assert_eq!(Ordinal::OmegaPow(3).render(), "ω^3");
        assert_eq!(Ordinal::Kappa(1).render(), "κ_1");
        assert_eq!(Ordinal::Kappa(2).render(), "κ_2");
    }

    #[test]
    fn convenience_constructors() {
        assert_eq!(Ordinal::omega_plus(0), Ordinal::Omega);
        assert_eq!(Ordinal::omega_plus(3), Ordinal::OmegaPlus(3));
        assert_eq!(Ordinal::omega_times(0), Ordinal::Finite(0));
        assert_eq!(Ordinal::omega_times(1), Ordinal::Omega);
        assert_eq!(Ordinal::omega_times(2), Ordinal::OmegaTimes(2));
        assert_eq!(Ordinal::kappa(1), Ordinal::Kappa(1));
    }

    #[test]
    fn le_includes_equality() {
        let omega = Ordinal::Omega;
        assert!(omega.le(&omega));
        assert!(omega.le(&Ordinal::OmegaPlus(1)));
        assert!(!Ordinal::OmegaPlus(1).le(&omega));
    }

    #[test]
    fn omega_squared_plus_finite_inner() {
        let a = Ordinal::OmegaSquaredPlus(Box::new(Ordinal::Finite(3)));
        let b = Ordinal::OmegaSquaredPlus(Box::new(Ordinal::Finite(7)));
        assert!(a.lt(&b));
        assert!(Ordinal::OmegaSquared.lt(&a));
    }

    #[test]
    fn sup_distributes_lt() {
        let sup = Ordinal::Sup(vec![
            Ordinal::Finite(2),
            Ordinal::Finite(5),
            Ordinal::Finite(10),
        ]);
        // sup < 100 iff all parts < 100
        assert!(sup.lt(&Ordinal::Finite(100)));
        // sup < 8 iff all parts < 8 — false because 10 ≥ 8
        assert!(!sup.lt(&Ordinal::Finite(8)));
    }

    #[test]
    fn cantor_normal_form_chain() {
        // 0 < 1 < ω < ω+1 < ω·2 < ω² < ω³ < κ_1 < κ_2
        let chain = vec![
            Ordinal::Finite(0),
            Ordinal::Finite(1),
            Ordinal::Omega,
            Ordinal::OmegaPlus(1),
            Ordinal::OmegaTimes(2),
            Ordinal::OmegaSquared,
            Ordinal::OmegaPow(3),
            Ordinal::Kappa(1),
            Ordinal::Kappa(2),
        ];
        for i in 0..chain.len() - 1 {
            assert!(
                chain[i].lt(&chain[i + 1]),
                "{} should be < {}",
                chain[i].render(),
                chain[i + 1].render()
            );
        }
    }

    #[test]
    fn render_via_display() {
        assert_eq!(format!("{}", Ordinal::Omega), "ω");
        assert_eq!(format!("{}", Ordinal::Kappa(1)), "κ_1");
    }

    // ----- Normalisation fix: OmegaPow(2) ≡ OmegaSquared -----

    #[test]
    fn normalise_omega_pow_2_equals_omega_squared() {
        let pow2 = Ordinal::OmegaPow(2);
        let squared = Ordinal::OmegaSquared;
        assert_eq!(pow2.normalize(), squared);
        // Critical: lt must be symmetric on equal-normalised values.
        assert!(!pow2.lt(&squared), "ω^2 must NOT be < ω²");
        assert!(!squared.lt(&pow2), "ω² must NOT be < ω^2");
    }

    #[test]
    fn normalise_idempotent() {
        let cases = vec![
            Ordinal::OmegaPow(0),
            Ordinal::OmegaPow(1),
            Ordinal::OmegaPow(2),
            Ordinal::OmegaTimes(0),
            Ordinal::OmegaTimes(1),
            Ordinal::OmegaPlus(0),
            Ordinal::OmegaTimesPlus { k: 1, n: 0 },
            Ordinal::OmegaTimesPlus { k: 1, n: 5 },
            Ordinal::OmegaTimesPlus { k: 3, n: 0 },
            Ordinal::Sup(vec![]),
            Ordinal::Sup(vec![Ordinal::Omega]),
        ];
        for c in &cases {
            let n1 = c.normalize();
            let n2 = n1.normalize();
            assert_eq!(n1, n2, "normalise must be idempotent on {:?}", c);
        }
    }

    #[test]
    fn normalise_collapses_degenerate_variants() {
        assert_eq!(Ordinal::OmegaPow(0).normalize(), Ordinal::Finite(1));
        assert_eq!(Ordinal::OmegaPow(1).normalize(), Ordinal::Omega);
        assert_eq!(Ordinal::OmegaTimes(0).normalize(), Ordinal::Finite(0));
        assert_eq!(Ordinal::OmegaTimes(1).normalize(), Ordinal::Omega);
        assert_eq!(Ordinal::OmegaPlus(0).normalize(), Ordinal::Omega);
        assert_eq!(
            Ordinal::OmegaTimesPlus { k: 1, n: 0 }.normalize(),
            Ordinal::Omega
        );
        assert_eq!(
            Ordinal::OmegaTimesPlus { k: 1, n: 7 }.normalize(),
            Ordinal::OmegaPlus(7)
        );
        assert_eq!(
            Ordinal::OmegaTimesPlus { k: 3, n: 0 }.normalize(),
            Ordinal::OmegaTimes(3)
        );
        assert_eq!(Ordinal::Sup(vec![]).normalize(), Ordinal::Finite(0));
        assert_eq!(
            Ordinal::Sup(vec![Ordinal::Omega]).normalize(),
            Ordinal::Omega
        );
    }

    #[test]
    fn lt_handles_omega_pow_2_correctly_after_fix() {
        // Before the normalise fix: OmegaSquared < OmegaPow(2) returned true (bug).
        // After: both are equal under lt (neither is < the other).
        let pow2 = Ordinal::OmegaPow(2);
        let squared = Ordinal::OmegaSquared;
        assert!(!pow2.lt(&squared));
        assert!(!squared.lt(&pow2));

        // And both are < OmegaPow(3).
        assert!(pow2.lt(&Ordinal::OmegaPow(3)));
        assert!(squared.lt(&Ordinal::OmegaPow(3)));

        // And both are < κ_1.
        assert!(pow2.lt(&Ordinal::Kappa(1)));
        assert!(squared.lt(&Ordinal::Kappa(1)));
    }
}
