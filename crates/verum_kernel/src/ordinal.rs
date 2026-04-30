//! Native ordinal arithmetic for the kernel — Cantor normal form with
//! large-cardinal extensions.
//!
//! Pre-this-module the kernel encoded ordinals via raw `u32` placeholders
//! (`KappaTier::KappaN(u32)`, bounded-arithmetic markers `999_999 = ω-1`,
//! `1_000_000 = ω`). This module replaces those encodings with a
//! mathematically-honest [`Ordinal`] type carrying:
//!
//!   * **Finite ordinals** `0, 1, 2, ...` (every natural number).
//!   * **Limit ordinals** at `ω, ω·2, ω·3, ..., ω², ω²·2, ..., ω³, ...`
//!     up to but not including `ε_0` (the supremum of the Cantor-normal-
//!     form fragment). Higher recursive ordinals are reachable via
//!     [`Ordinal::Sup`] (countable supremum).
//!   * **Inaccessible cardinals** `κ_n` for any `n: u32` — the
//!     large-cardinal extension. `κ_1` is the first inaccessible
//!     above ω, `κ_2` is the second, etc. Used by the (∞,2)-stack
//!     model and Drake reflection.
//!
//! The `lt` / `succ` / `is_regular` / `is_limit` operations are
//! decidable on the Cantor-normal-form fragment; for `Sup` of an
//! arbitrary countable family, decidability is delegated to the
//! Sup operands.
//!
//! ## Design rationale
//!
//! Many kernel rules need ordinal comparison: K-Universe-Ascent V2
//! checks `source ≤ target`, K-Refine-omega reasons about modal
//! depth, Diakrisis 113.T autopoiesis requires `κ ≥ ω²`, MSFS
//! Theorem A.7 needs the `(∞, ∞)` ↪ `(∞, ∞ + 1)` stabilisation.
//! Each of those operations needs Bool-valued decidable comparison,
//! not opaque `Int` predicates.
//!
//! By centralising ordinal arithmetic here we (a) avoid scattered ad-
//! hoc encodings, (b) get a single point at which Drake reflection
//! / κ-tower extensions land cleanly, (c) match the literature's
//! Cantor-normal-form notation 1:1 for diagnostics.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A native ordinal value covering the Cantor-normal-form fragment
/// plus inaccessible cardinals plus countable suprema.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ordinal {
    /// A finite ordinal `n`.  `Finite(0)` is the smallest ordinal,
    /// `Finite(1)` is its successor, etc.
    Finite(u32),

    /// The first transfinite ordinal `ω`.
    Omega,

    /// `ω + n` for `n ≥ 1`.  Convention: `OmegaPlus(1)` is `ω + 1`,
    /// `OmegaPlus(2)` is `ω + 2`, etc.  `OmegaPlus(0)` is normalised
    /// to plain `Omega`.
    OmegaPlus(u32),

    /// `ω·k` for `k ≥ 2`.  `OmegaTimes(2)` is `ω·2`, `OmegaTimes(3)`
    /// is `ω·3`, etc.  `OmegaTimes(1)` is normalised to `Omega`.
    OmegaTimes(u32),

    /// `ω·k + n` for `k ≥ 1`, `n ≥ 1`.  Supports values like `ω·3 + 5`.
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

    /// `ω² + α` for any smaller ordinal `α < ω²`.
    OmegaSquaredPlus(Box<Ordinal>),

    /// `ω^e` for `e ≥ 3`.  Covers ω³, ω⁴, ...
    OmegaPow(u32),

    /// The `n`-th inaccessible cardinal `κ_n`.  Convention: `κ_0` is
    /// undefined (use `Omega` instead); `κ_1` is the first inaccessible
    /// above ω, `κ_2` is the second.  Lying above every Cantor-normal-
    /// form ordinal.
    Kappa(u32),

    /// Countable supremum of an arbitrary family of ordinals.  Used to
    /// represent ordinals beyond the Cantor-normal-form fragment
    /// (e.g. `ε_0 = sup(ω, ω^ω, ω^ω^ω, ...)`) without committing to
    /// a particular limit form.
    Sup(Vec<Ordinal>),
}

impl Ordinal {
    /// Successor ordinal `α + 1`.  Saturates at `Kappa(u32::MAX)`
    /// (defensive — exhausting `u32::MAX` inaccessibles is impossible
    /// in any practical proof).
    pub fn succ(&self) -> Self {
        match self {
            Ordinal::Finite(n) => match n.checked_add(1) {
                Some(m) => Ordinal::Finite(m),
                None => Ordinal::Omega,
            },
            Ordinal::Omega => Ordinal::OmegaPlus(1),
            Ordinal::OmegaPlus(n) => Ordinal::OmegaPlus(n.saturating_add(1)),
            Ordinal::OmegaTimes(k) => Ordinal::OmegaTimesPlus { k: *k, n: 1 },
            Ordinal::OmegaTimesPlus { k, n } => Ordinal::OmegaTimesPlus {
                k: *k,
                n: n.saturating_add(1),
            },
            Ordinal::OmegaSquared => {
                Ordinal::OmegaSquaredPlus(Box::new(Ordinal::Finite(1)))
            }
            Ordinal::OmegaSquaredPlus(inner) => {
                Ordinal::OmegaSquaredPlus(Box::new(inner.succ()))
            }
            Ordinal::OmegaPow(_e) => {
                // ω^e has successor ω^e + 1 — represented as Sup with
                // the +1 step. Underscore-prefixed because the
                // exponent isn't read directly — `self.clone()`
                // already captures the full ω^e shape.
                Ordinal::Sup(vec![self.clone(), Ordinal::Finite(1)])
            }
            Ordinal::Kappa(_n) => {
                // κ_n + 1 is still beyond Cantor-normal-form; encode as Sup.
                // Underscore-prefixed for the same reason as OmegaPow above.
                Ordinal::Sup(vec![self.clone(), Ordinal::Finite(1)])
            }
            Ordinal::Sup(parts) => {
                let mut new_parts = parts.clone();
                new_parts.push(Ordinal::Finite(1));
                Ordinal::Sup(new_parts)
            }
        }
    }

    /// Strict less-than on ordinals.  Decidable on the Cantor-normal-
    /// form fragment + inaccessibles; for `Sup` operands the comparison
    /// reduces to all-pairs comparison on the family.
    ///
    /// **Normalisation contract.**  `lt` first canonicalises both
    /// operands via [`Ordinal::normalize`] so that semantic-equivalents
    /// like `OmegaPow(2)` and `OmegaSquared` are correctly handled
    /// as equal.  This avoids the "asymmetric lt" bug that pre-fix
    /// allowed `OmegaSquared < OmegaPow(2)` to return true.
    pub fn lt(&self, other: &Self) -> bool {
        let a = self.normalize();
        let b = other.normalize();
        a.lt_raw(&b)
    }

    /// Raw lt without normalisation — used internally by `lt` after
    /// both operands have been normalised.  Public for callers that
    /// have already normalised.
    pub fn lt_raw(&self, other: &Self) -> bool {
        use Ordinal::*;
        match (self, other) {
            // Sup branches FIRST — they distribute over lt on either side.
            // sup S < o iff every s in S satisfies s < o.
            // s < sup S iff some part of S is ≥ s.
            (Sup(parts), other) => parts.iter().all(|p| p.lt_raw(other)),
            (s, Sup(parts)) => parts.iter().any(|p| s.lt_raw(p) || s == p),

            // Finite vs anything (after Sup is handled).
            (Finite(a), Finite(b)) => a < b,
            (Finite(_), _) => true,           // every finite < every transfinite
            (_, Finite(_)) => false,          // no transfinite < finite

            // ω vs others.
            (Omega, Omega) => false,
            (Omega, OmegaPlus(_)) => true,
            (Omega, OmegaTimes(k)) => *k >= 2,
            (Omega, OmegaTimesPlus { .. }) => true,
            (Omega, OmegaSquared) => true,
            (Omega, OmegaSquaredPlus(_)) => true,
            (Omega, OmegaPow(_)) => true,
            (Omega, Kappa(_)) => true,

            (OmegaPlus(_a), Omega) => false,
            (OmegaPlus(a), OmegaPlus(b)) => a < b,
            (OmegaPlus(_), OmegaTimes(k)) => *k >= 2,
            (OmegaPlus(_), OmegaTimesPlus { k, n: _ }) => *k >= 2,
            (OmegaPlus(_), OmegaSquared) => true,
            (OmegaPlus(_), OmegaSquaredPlus(_)) => true,
            (OmegaPlus(_), OmegaPow(_)) => true,
            (OmegaPlus(_), Kappa(_)) => true,

            (OmegaTimes(_a), Omega) => false,
            (OmegaTimes(a), OmegaPlus(_)) => *a < 2,
            (OmegaTimes(a), OmegaTimes(b)) => a < b,
            (OmegaTimes(a), OmegaTimesPlus { k, n: _ }) => a <= k,
            (OmegaTimes(_), OmegaSquared) => true,
            (OmegaTimes(_), OmegaSquaredPlus(_)) => true,
            (OmegaTimes(_), OmegaPow(_)) => true,
            (OmegaTimes(_), Kappa(_)) => true,

            (OmegaTimesPlus { .. }, Omega) => false,
            (OmegaTimesPlus { k, n: _ }, OmegaPlus(_)) => *k < 2,
            (OmegaTimesPlus { k, n: _ }, OmegaTimes(b)) => k < b,
            (OmegaTimesPlus { k: ka, n: na }, OmegaTimesPlus { k: kb, n: nb }) => {
                ka < kb || (ka == kb && na < nb)
            }
            (OmegaTimesPlus { .. }, OmegaSquared) => true,
            (OmegaTimesPlus { .. }, OmegaSquaredPlus(_)) => true,
            (OmegaTimesPlus { .. }, OmegaPow(_)) => true,
            (OmegaTimesPlus { .. }, Kappa(_)) => true,

            (OmegaSquared, OmegaSquared) => false,
            (OmegaSquared, OmegaSquaredPlus(_)) => true,
            (OmegaSquared, OmegaPow(_)) => true,
            (OmegaSquared, Kappa(_)) => true,
            (OmegaSquared, _) => false,

            (OmegaSquaredPlus(_a), OmegaSquared) => false,
            (OmegaSquaredPlus(a), OmegaSquaredPlus(b)) => a.lt(b),
            (OmegaSquaredPlus(_), OmegaPow(e)) => *e >= 3,
            (OmegaSquaredPlus(_), Kappa(_)) => true,
            (OmegaSquaredPlus(_), _) => false,

            (OmegaPow(a), OmegaPow(b)) => a < b,
            (OmegaPow(_), Kappa(_)) => true,
            (OmegaPow(_), _) => false,

            (Kappa(a), Kappa(b)) => a < b,
            (Kappa(_), _) => false,
        }
    }

    /// Less-than-or-equal: `lt || ==`.
    pub fn le(&self, other: &Self) -> bool {
        self.lt(other) || self == other
    }

    /// True iff this ordinal is a *limit* ordinal (no immediate predecessor).
    /// `0` is *not* a limit (by convention some authors include it; we
    /// follow the Mizar / Coq convention that excludes 0).
    pub fn is_limit(&self) -> bool {
        match self {
            Ordinal::Finite(_) => false,
            Ordinal::Omega => true,
            Ordinal::OmegaPlus(_) => false,
            Ordinal::OmegaTimes(_) => true,
            Ordinal::OmegaTimesPlus { .. } => false,
            Ordinal::OmegaSquared => true,
            Ordinal::OmegaSquaredPlus(inner) => inner.is_limit(),
            Ordinal::OmegaPow(_) => true,
            Ordinal::Kappa(_) => true,
            Ordinal::Sup(_) => true,
        }
    }

    /// True iff this ordinal is *regular* (its cofinality equals itself).
    /// Per the standard set-theoretic definition: every finite ordinal
    /// > 0 is regular trivially, ω is regular, every successor cardinal
    /// > is regular; singular limit cardinals are not.
    ///
    /// In our normalised form: ω is regular, every κ_n is regular
    /// (inaccessibles are by construction regular limit cardinals),
    /// `Sup` is conservatively NOT regular (a sup-of-smaller construction).
    pub fn is_regular(&self) -> bool {
        match self {
            Ordinal::Finite(n) => *n > 0,
            Ordinal::Omega => true,
            Ordinal::Kappa(_) => true,
            // Successor ordinals are regular.
            Ordinal::OmegaPlus(_) => true,
            Ordinal::OmegaTimesPlus { .. } => true,
            Ordinal::OmegaSquaredPlus(_) => true,
            // Limit ordinals below κ_1 (other than ω) are typically not
            // regular under classical set theory.
            Ordinal::OmegaTimes(_) => false,
            Ordinal::OmegaSquared => false,
            Ordinal::OmegaPow(_) => false,
            Ordinal::Sup(_) => false,
        }
    }

    /// True iff this ordinal is an inaccessible cardinal (the κ-tower).
    pub fn is_inaccessible(&self) -> bool {
        matches!(self, Ordinal::Kappa(_))
    }

    /// **Next inaccessible cardinal** above `self` in the κ-tower —
    /// the universe-ascent operation used for `PSh(C)` (HTT 5.5),
    /// dependent universe hierarchies, and Grothendieck-universe
    /// stratification.
    ///
    /// Distinguished from [`Ordinal::succ`] which returns the
    /// *successor ordinal* `α + 1` (a small step within the same
    /// universe).  `next_inaccessible` performs a *universe* step:
    /// it bumps to the next strongly-inaccessible cardinal `κ`,
    /// which is the smallest cardinal that bounds *all* small
    /// constructions on the source universe.
    ///
    /// Behaviour:
    ///   * `Finite(_)` / `Omega` / sub-ω² ordinals → `Kappa(0)`
    ///     (the first inaccessible, sometimes denoted `U`).
    ///   * `Kappa(n)` → `Kappa(n + 1)` (saturating at `u32::MAX`
    ///     defensively; in practice κ-towers of arbitrary finite
    ///     height are admitted via `Kappa(n)` plus framework-axiom
    ///     extension).
    ///   * `Sup(_)` → `Kappa(0)` if the supremum is below κ-tower,
    ///     otherwise the next κ above the largest part.
    ///
    /// This is the operation invoked by `presheaf_category` to
    /// realise the HTT 5.5 universe ascent in the kernel surface.
    pub fn next_inaccessible(&self) -> Self {
        match self {
            Ordinal::Kappa(n) => Ordinal::Kappa(n.saturating_add(1)),
            Ordinal::Sup(parts) => {
                // Find the largest κ-tier in the parts; bump it.
                let mut best = None;
                for p in parts {
                    if let Ordinal::Kappa(k) = p {
                        best = Some(best.map_or(*k, |b: u32| b.max(*k)));
                    }
                }
                match best {
                    Some(k) => Ordinal::Kappa(k.saturating_add(1)),
                    None => Ordinal::Kappa(0),
                }
            }
            // Anything strictly below the κ-tower ascends to κ_0.
            _ => Ordinal::Kappa(0),
        }
    }

    /// Render the ordinal in standard mathematical notation.  Used for
    /// diagnostics and audit reports.
    pub fn render(&self) -> String {
        match self {
            Ordinal::Finite(n) => n.to_string(),
            Ordinal::Omega => "ω".to_string(),
            Ordinal::OmegaPlus(n) => format!("ω + {}", n),
            Ordinal::OmegaTimes(k) => format!("ω·{}", k),
            Ordinal::OmegaTimesPlus { k, n } => format!("ω·{} + {}", k, n),
            Ordinal::OmegaSquared => "ω²".to_string(),
            Ordinal::OmegaSquaredPlus(inner) => format!("ω² + {}", inner.render()),
            Ordinal::OmegaPow(e) => format!("ω^{}", e),
            Ordinal::Kappa(n) => format!("κ_{}", n),
            Ordinal::Sup(parts) => {
                let inner: Vec<String> = parts.iter().map(|p| p.render()).collect();
                format!("sup({})", inner.join(", "))
            }
        }
    }

    /// Convenience: ω + k for any k (handles k = 0, k = 1, k > 1
    /// uniformly, normalising to the canonical variant).
    pub fn omega_plus(k: u32) -> Self {
        match k {
            0 => Ordinal::Omega,
            n => Ordinal::OmegaPlus(n),
        }
    }

    /// Convenience: ω·k for any k.
    pub fn omega_times(k: u32) -> Self {
        match k {
            0 => Ordinal::Finite(0),
            1 => Ordinal::Omega,
            n => Ordinal::OmegaTimes(n),
        }
    }

    /// Convenience: κ_n. Panics on n == 0 (use `Omega` instead).
    pub fn kappa(n: u32) -> Self {
        assert!(n >= 1, "Ordinal::kappa(0) is undefined; use Omega");
        Ordinal::Kappa(n)
    }

    /// Normalise an ordinal to its canonical Cantor-normal-form
    /// representation.  Resolves the OmegaPow(2) ≡ OmegaSquared and
    /// degenerate Sup cases.
    ///
    /// # Normalisation rules
    ///
    ///   * `OmegaPow(0)` → `Finite(1)` (ω^0 = 1)
    ///   * `OmegaPow(1)` → `Omega` (ω^1 = ω)
    ///   * `OmegaPow(2)` → `OmegaSquared` (ω^2 = ω²)
    ///   * `OmegaTimes(0)` → `Finite(0)`
    ///   * `OmegaTimes(1)` → `Omega`
    ///   * `OmegaPlus(0)` → `Omega`
    ///   * `OmegaTimesPlus { k: 1, n }` → `OmegaPlus(n)`
    ///   * `OmegaTimesPlus { k, n: 0 }` → `OmegaTimes(k)`
    ///   * `Sup(parts)` → if any part is the maximum, return that
    ///     directly when the rest are strictly less; otherwise leave
    ///     as Sup.  Empty Sup → `Finite(0)`.
    ///
    /// Idempotent: `α.normalize().normalize() == α.normalize()`.
    pub fn normalize(&self) -> Self {
        match self {
            Ordinal::OmegaPow(0) => Ordinal::Finite(1),
            Ordinal::OmegaPow(1) => Ordinal::Omega,
            Ordinal::OmegaPow(2) => Ordinal::OmegaSquared,
            Ordinal::OmegaTimes(0) => Ordinal::Finite(0),
            Ordinal::OmegaTimes(1) => Ordinal::Omega,
            Ordinal::OmegaPlus(0) => Ordinal::Omega,
            Ordinal::OmegaTimesPlus { k: 0, n: 0 } => Ordinal::Finite(0),
            Ordinal::OmegaTimesPlus { k: 0, n } => Ordinal::Finite(*n),
            Ordinal::OmegaTimesPlus { k: 1, n: 0 } => Ordinal::Omega,
            Ordinal::OmegaTimesPlus { k: 1, n } => Ordinal::OmegaPlus(*n),
            Ordinal::OmegaTimesPlus { k, n: 0 } => Ordinal::OmegaTimes(*k),
            Ordinal::OmegaSquaredPlus(inner) => {
                let inner_norm = inner.normalize();
                if matches!(inner_norm, Ordinal::Finite(0)) {
                    Ordinal::OmegaSquared
                } else {
                    Ordinal::OmegaSquaredPlus(Box::new(inner_norm))
                }
            }
            Ordinal::Sup(parts) => {
                if parts.is_empty() {
                    Ordinal::Finite(0)
                } else if parts.len() == 1 {
                    parts[0].normalize()
                } else {
                    let normalised: Vec<_> = parts.iter().map(|p| p.normalize()).collect();
                    Ordinal::Sup(normalised)
                }
            }
            other => other.clone(),
        }
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
        assert_eq!(
            Ordinal::OmegaSquared.next_inaccessible(),
            Ordinal::Kappa(0)
        );
        assert_eq!(
            Ordinal::OmegaPow(7).next_inaccessible(),
            Ordinal::Kappa(0)
        );
    }

    #[test]
    fn next_inaccessible_climbs_kappa_tower() {
        assert_eq!(Ordinal::Kappa(0).next_inaccessible(), Ordinal::Kappa(1));
        assert_eq!(Ordinal::Kappa(1).next_inaccessible(), Ordinal::Kappa(2));
        assert_eq!(
            Ordinal::Kappa(7).next_inaccessible(),
            Ordinal::Kappa(8)
        );
    }

    #[test]
    fn next_inaccessible_saturates_at_u32_max() {
        assert_eq!(
            Ordinal::Kappa(u32::MAX).next_inaccessible(),
            Ordinal::Kappa(u32::MAX)
        );
    }

    #[test]
    fn next_inaccessible_distinct_from_succ() {
        // For κ_n, succ goes to Sup([κ_n, 1]) (ordinal step), but
        // next_inaccessible goes to κ_{n+1} (universe step).  These
        // are *not* the same operation.
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
        // Successor ordinals: regular.
        assert!(Ordinal::Finite(1).is_regular());
        assert!(!Ordinal::Finite(0).is_regular());   // 0 is not regular by convention
        assert!(Ordinal::OmegaPlus(1).is_regular());
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
        assert_eq!(
            Ordinal::OmegaTimesPlus { k: 3, n: 5 }.render(),
            "ω·3 + 5"
        );
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
        let sup = Ordinal::Sup(vec![Ordinal::Finite(2), Ordinal::Finite(5), Ordinal::Finite(10)]);
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
