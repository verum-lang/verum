//! Categorical coherence K-Universe-Ascent kernel rule + UniverseTier — split per #198.
//!

//! Verifies that meta-classifier applications `M_stack(α)` ascend
//! Grothendieck-universe levels per the canonical κ-tower of
//! Theorem 131.T (∞,2)-stack model. Per Theorem 134.T (tight 2-inacc
//! bound), only two non-trivial universe levels are needed; the
//! `Truncated` marker is reserved for the Cat-baseline strictly
//! below κ_1.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::KernelError;
use crate::diakrisis_bridge::{BridgeAudit, admit_drake_reflection_extended};

/// Universe level for K-Universe-Ascent (Theorem 131.T (∞,2)-stack
/// model). Per Theorem 134.T (tight 2-inacc bound), only two
/// non-trivial Grothendieck-universe levels are needed; the
/// `Truncated` marker is reserved for the Cat-baseline that lives
/// strictly below κ_1.
///

/// Mirrors `core.math.stack_model::Universe` (single source of truth
/// between kernel and stdlib).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UniverseTier {
    /// Cat-baseline: only set-level objects. The canonical
    /// truncation `truncate(stack_model, level=2, universe=κ_1)`.
    Truncated,
    /// First Grothendieck universe (κ_1-inaccessible).
    Kappa1,
    /// Second Grothendieck universe (κ_2-inaccessible). The stack-
    /// model meta-classifier ascends κ_1 → κ_2 (Lemma 131.L1)
    /// and stabilises here via Drake reflection (Lemma 131.L3) —
    /// no κ_3 needed.
    Kappa2,
}

/// Per-variant projection for [`UniverseTier`].
///
/// `rank` makes the strict ordering Truncated < κ_1 < κ_2 explicit
/// and dense; the legacy `lt` rode a 6-arm match against the variant
/// declaration order. `name` is the canonical text rendering for
/// diagnostic surfaces.
///
/// Per Theorem 134.T (tight 2-inacc bound), only two non-trivial
/// Grothendieck-universe levels are needed; the `Truncated` marker
/// is reserved for the Cat-baseline that lives strictly below κ_1.
/// The succ-saturation invariant `succ(Kappa2) == Kappa2` (Lemma
/// 131.L3 Drake-reflection closure) is structurally enforced via
/// the `successor_rank` field — it equals `rank` for `Kappa2`,
/// `rank + 1` otherwise.
#[derive(Debug, Clone, Copy)]
pub struct UniverseTierMeta {
    pub name: &'static str,
    pub rank: u8,
    pub successor_rank: u8,
}

impl UniverseTier {
    pub const ALL: &'static [Self] = &[Self::Truncated, Self::Kappa1, Self::Kappa2];

    pub const fn meta(self) -> UniverseTierMeta {
        match self {
            Self::Truncated => UniverseTierMeta {
                name: "truncated",
                rank: 0,
                successor_rank: 1,
            },
            Self::Kappa1 => UniverseTierMeta {
                name: "κ_1",
                rank: 1,
                successor_rank: 2,
            },
            Self::Kappa2 => UniverseTierMeta {
                name: "κ_2",
                // Lemma 131.L3 / Theorem 134.T: succ saturates at
                // κ_2 — Drake reflection closes the ascent here.
                rank: 2,
                successor_rank: 2,
            },
        }
    }

    /// Canonical text rendering for diagnostic surfaces.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Parse a tier name back to the typed form. Accepts the
    /// canonical kebab-form (`"truncated"`) and the κ-glyph forms
    /// (`"κ_1"`, `"κ_2"`). Closes a drift defect: previously
    /// `as_str` was present but no inverse mapping existed.
    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// Universe-ladder rank: Truncated=0, κ_1=1, κ_2=2.
    #[inline]
    pub const fn rank(&self) -> u8 {
        self.meta().rank
    }

    /// Strict universe ordering: Truncated < κ_1 < κ_2.
    /// Collapses to a single rank comparison rather than a 6-arm
    /// match.
    #[inline]
    pub const fn lt(&self, other: &Self) -> bool {
        self.rank() < other.rank()
    }

    /// Successor: Truncated → κ_1 → κ_2 → κ_2 (saturates at the top
    /// per Lemma 131.L3 / Theorem 134.T tight-bound). Drift-pinned
    /// in the test module.
    #[inline]
    pub const fn succ(&self) -> Self {
        // Linear scan over ALL by `successor_rank` — tiny domain
        // (3 variants), const-evaluable.
        let target = self.meta().successor_rank;
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i].meta().rank == target {
                return Self::ALL[i];
            }
            i += 1;
        }
        // Unreachable: succ_rank is always one of {0, 1, 2}.
        *self
    }
}

/// Categorical coherence — `K-Universe-Ascent` kernel rule.
///

/// Verifies that a meta-classifier application `M_stack(α)`
/// correctly ascends the universe level by exactly one step:
///

/// ```text
///  Γ ⊢ α : Articulation@U_k Γ ⊢ M_stack(α) : Articulation@U_{k+1}
///  ──────────────────────────────────────────────────────────────────── (K-Universe-Ascent)
///  Γ ⊢ M_stack : Functor[Articulation@U_k → Articulation@U_{k+1}]
/// ```
///

/// Per Lemma 131.L1 (universe-ascent): M_stack(F: U_1) ∈ U_2.
/// Per Lemma 131.L3 (Drake-reflection closure): M_stack(F: U_2)
/// stays in U_2; no κ_3 is needed.
///

/// The rule rejects:
///  - source/target tier inversion (target tier < source tier);
///  - source = Truncated with target ≥ Kappa1 — Truncated is the
///  Cat-baseline; meta-classifier application must start from
///  κ_1 or κ_2 per Theorem 131.T;
///  - source = Kappa2 with target = Kappa1 — would violate the
///  tight bound;
/// and accepts:
///  - source = κ_1, target = κ_2 (the canonical ascent);
///  - source = κ_2, target = κ_2 (Drake-reflection closure);
///  - source = Truncated, target = Truncated (Cat-baseline
///  identity, no ascent claimed).
pub fn check_universe_ascent(
    source: UniverseTier,
    target: UniverseTier,
    context: &str,
) -> Result<(), KernelError> {
    // Truncated identity — no ascent, no error.
    if source == UniverseTier::Truncated && target == UniverseTier::Truncated {
        return Ok(());
    }
    // Truncated → ≥κ_1 — meta-classifier must not start from the
    // Cat-baseline; the user should have lifted to κ_1 first.
    if source == UniverseTier::Truncated && target != UniverseTier::Truncated {
        return Err(KernelError::UniverseAscentInvalid {
            context: Text::from(context),
            from_tier: Text::from(source.as_str()),
            to_tier: Text::from(target.as_str()),
        });
    }
    // κ_1 → κ_2 — canonical ascent (Lemma 131.L1).
    if source == UniverseTier::Kappa1 && target == UniverseTier::Kappa2 {
        return Ok(());
    }
    // κ_2 → κ_2 — Drake-reflection closure (Lemma 131.L3).
    if source == UniverseTier::Kappa2 && target == UniverseTier::Kappa2 {
        return Ok(());
    }
    // κ_1 → κ_1 — Cat-baseline-style, no ascent. Acceptable for
    // identity meta-classifier.
    if source == UniverseTier::Kappa1 && target == UniverseTier::Kappa1 {
        return Ok(());
    }
    // Anything else (κ_2 → κ_1, κ_? → Truncated when source > Truncated):
    // tier inversion or out-of-bound; reject.
    Err(KernelError::UniverseAscentInvalid {
        context: Text::from(context),
        from_tier: Text::from(source.as_str()),
        to_tier: Text::from(target.as_str()),
    })
}

// =============================================================================
// V2: arbitrary κ-tower with extended Drake reflection bridge admit.
// =============================================================================

/// V2 universe level — `Truncated` plus an indexed `KappaN(n)` for
/// any inaccessible level n ≥ 1. Strictly more expressive than
/// [`UniverseTier`]; preserved-by-conversion from the V0 enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KappaTier {
    /// Cat-baseline (set-level only) — same as `UniverseTier::Truncated`.
    Truncated,
    /// `κ_n` inaccessible level for `n ≥ 1`.
    KappaN(u32),
}

impl KappaTier {
    /// Render for diagnostics. `KappaN(1)` → "κ_1", `KappaN(2)` → "κ_2",
    /// etc. Truncated is "truncated".
    pub fn render(&self) -> String {
        match self {
            KappaTier::Truncated => "truncated".to_string(),
            KappaTier::KappaN(n) => format!("κ_{}", n),
        }
    }

    /// Strict ordering: Truncated < κ_n for any n; κ_a < κ_b iff a < b.
    pub fn lt(&self, other: &Self) -> bool {
        match (self, other) {
            (KappaTier::Truncated, KappaTier::KappaN(_)) => true,
            (KappaTier::Truncated, KappaTier::Truncated) => false,
            (KappaTier::KappaN(_), KappaTier::Truncated) => false,
            (KappaTier::KappaN(a), KappaTier::KappaN(b)) => a < b,
        }
    }

    /// Successor: Truncated → κ_1; κ_n → κ_{n+1}; saturates at u32::MAX.
    pub fn succ(&self) -> Self {
        match self {
            KappaTier::Truncated => KappaTier::KappaN(1),
            KappaTier::KappaN(n) => KappaTier::KappaN(n.saturating_add(1)),
        }
    }

    /// Project into the native [`crate::ordinal::Ordinal`] type.
    /// Truncated → `Ordinal::Finite(0)` (the smallest ordinal); KappaN(n)
    /// → `Ordinal::Kappa(n)`. Used by callers that want the unified
    /// ordinal-arithmetic surface (regularity / inaccessibility checks /
    /// arbitrary ordinal comparison).
    pub fn to_ordinal(&self) -> crate::ordinal::Ordinal {
        match self {
            KappaTier::Truncated => crate::ordinal::Ordinal::Finite(0),
            KappaTier::KappaN(n) => crate::ordinal::Ordinal::Kappa(*n),
        }
    }

    /// Construct a `KappaTier` from an `Ordinal`, returning `None` if
    /// the ordinal isn't representable in the V2 KappaTier surface
    /// (only `Finite(0)` for Truncated and `Kappa(n)` for KappaN(n)
    /// are admitted). Use `Ordinal::normalize` before this call to
    /// canonicalise inputs.
    pub fn from_ordinal(o: &crate::ordinal::Ordinal) -> Option<Self> {
        let normalised = o.normalize();
        match normalised {
            crate::ordinal::Ordinal::Finite(0) => Some(KappaTier::Truncated),
            crate::ordinal::Ordinal::Kappa(n) => Some(KappaTier::KappaN(n)),
            _ => None,
        }
    }

    /// Convenience: is this κ-tier regular? Truncated and every
    /// `KappaN(n)` are regular by construction (inaccessibility ⟹
    /// regularity). Routes through `Ordinal::is_regular`.
    pub fn is_regular(&self) -> bool {
        // Truncated (Finite(0)) is NOT regular per Ordinal convention,
        // but KappaTier::Truncated represents a degenerate "no-κ" state
        // that's special-cased rather than projected to Finite(0).
        // We use the kappa-projection only for KappaN.
        match self {
            KappaTier::Truncated => false,
            KappaTier::KappaN(_) => true, // every inaccessible is regular
        }
    }

    /// True iff this tier is an inaccessible cardinal (a `KappaN(_)`).
    pub fn is_inaccessible(&self) -> bool {
        matches!(self, KappaTier::KappaN(_))
    }
}

impl From<UniverseTier> for KappaTier {
    fn from(t: UniverseTier) -> Self {
        match t {
            UniverseTier::Truncated => Self::Truncated,
            UniverseTier::Kappa1 => Self::KappaN(1),
            UniverseTier::Kappa2 => Self::KappaN(2),
        }
    }
}

/// V2 universe-ascent rule with audit-trail-aware Drake reflection
/// admit. Strictly stronger than [`check_universe_ascent`]:
///

///  * Truncated identity / Truncated → ≥κ_1 mismatch / canonical
///  κ_1 → κ_2 ascent / Drake closure at κ_2 / κ_1 → κ_1 are
///  decided directly (empty audit).
///

///  * Ascents involving κ_n for n ≥ 3, OR multi-step ascents
///  (target tier strictly more than one level above source)
///  are admitted via `BridgeId::DrakeReflectionExtended`.
///  The structural algorithm beyond κ_2 is preprint-blocked on
///  Diakrisis Lemma 131.L4.
///

///  * Tier inversion (target < source) is rejected uniformly,
///  regardless of tier index.
///

/// **Soundness invariant**: V2 never widens V0's accept set on
/// the {Truncated, κ_1, κ_2} input domain. New ascent classes
/// reachable in V2 strictly require the bridge admit.
pub fn check_universe_ascent_v2(
    source: KappaTier,
    target: KappaTier,
    audit: &mut BridgeAudit,
    context: &str,
) -> Result<(), KernelError> {
    // Truncated identity — no ascent.
    if source == KappaTier::Truncated && target == KappaTier::Truncated {
        return Ok(());
    }
    // Truncated → ≥ κ_1: Cat-baseline must lift through κ_1 explicitly.
    if source == KappaTier::Truncated {
        return Err(KernelError::UniverseAscentInvalid {
            context: Text::from(context),
            from_tier: Text::from(source.render()),
            to_tier: Text::from(target.render()),
        });
    }
    // ≥ κ_1 → Truncated: tier-inversion.
    if target == KappaTier::Truncated {
        return Err(KernelError::UniverseAscentInvalid {
            context: Text::from(context),
            from_tier: Text::from(source.render()),
            to_tier: Text::from(target.render()),
        });
    }
    let (s, t) = match (source, target) {
        (KappaTier::KappaN(s), KappaTier::KappaN(t)) => (s, t),
        _ => unreachable!("Truncated cases handled above"),
    };
    // Tier inversion at κ-level.
    if t < s {
        return Err(KernelError::UniverseAscentInvalid {
            context: Text::from(context),
            from_tier: Text::from(source.render()),
            to_tier: Text::from(target.render()),
        });
    }
    // decidable cases: only the {1, 2} domain has a structural
    // algorithm (Lemma 131.L1 + Lemma 131.L3 + Theorem 134.T tight
    // bound). Everything else needs the Drake-extended admit.
    let is_v0_pair = matches!(
        (s, t),
        (1, 2) |  // canonical ascent (Lemma 131.L1)
        (2, 2) |  // Drake reflection terminus (Lemma 131.L3)
        (1, 1) // κ_1 → κ_1 reflexive
    );
    if is_v0_pair {
        return Ok(());
    }
    // Anything else with s ≤ t and both ≥ 1: admits via Drake-extended
    // bridge. This covers κ_n → κ_n for n ≥ 3 (extended reflection),
    // κ_n → κ_{n+1} for n ≥ 2 (extended ascent), and multi-step
    // jumps κ_s → κ_t with t > s+1 (Diakrisis 131.L4 closure).
    admit_drake_reflection_extended(audit, context);
    Ok(())
}

#[cfg(test)]
mod v2_tests {
    use super::*;

    #[test]
    fn level_renders_kappa_n() {
        assert_eq!(KappaTier::Truncated.render(), "truncated");
        assert_eq!(KappaTier::KappaN(1).render(), "κ_1");
        assert_eq!(KappaTier::KappaN(7).render(), "κ_7");
    }

    #[test]
    fn level_succ_advances_correctly() {
        assert_eq!(KappaTier::Truncated.succ(), KappaTier::KappaN(1));
        assert_eq!(KappaTier::KappaN(1).succ(), KappaTier::KappaN(2));
        assert_eq!(KappaTier::KappaN(7).succ(), KappaTier::KappaN(8));
    }

    #[test]
    fn level_succ_saturates_at_u32_max() {
        let max = KappaTier::KappaN(u32::MAX);
        assert_eq!(max.succ(), max, "succ must saturate at u32::MAX");
    }

    #[test]
    fn level_lt_strict_ordering() {
        assert!(KappaTier::Truncated.lt(&KappaTier::KappaN(1)));
        assert!(KappaTier::KappaN(1).lt(&KappaTier::KappaN(2)));
        assert!(KappaTier::KappaN(2).lt(&KappaTier::KappaN(7)));
        assert!(!KappaTier::KappaN(2).lt(&KappaTier::KappaN(2)));
        assert!(!KappaTier::KappaN(2).lt(&KappaTier::KappaN(1)));
    }

    #[test]
    fn from_universe_tier_preserves_semantics() {
        assert_eq!(
            KappaTier::from(UniverseTier::Truncated),
            KappaTier::Truncated
        );
        assert_eq!(KappaTier::from(UniverseTier::Kappa1), KappaTier::KappaN(1));
        assert_eq!(KappaTier::from(UniverseTier::Kappa2), KappaTier::KappaN(2));
    }

    #[test]
    fn v2_admits_v0_pairs_with_empty_audit() {
        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::KappaN(1),
            KappaTier::KappaN(2),
            &mut a,
            "κ_1→κ_2",
        )
        .unwrap();
        assert!(a.is_decidable());

        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::KappaN(2),
            KappaTier::KappaN(2),
            &mut a,
            "κ_2 Drake",
        )
        .unwrap();
        assert!(a.is_decidable());

        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::KappaN(1),
            KappaTier::KappaN(1),
            &mut a,
            "κ_1 reflexive",
        )
        .unwrap();
        assert!(a.is_decidable());

        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::Truncated,
            KappaTier::Truncated,
            &mut a,
            "Truncated reflexive",
        )
        .unwrap();
        assert!(a.is_decidable());
    }

    #[test]
    fn v2_admits_higher_kappa_via_drake_extended() {
        // κ_3 → κ_3 — beyond Theorem 134.T's tight bound.
        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::KappaN(3),
            KappaTier::KappaN(3),
            &mut a,
            "κ_3 reflexive",
        )
        .unwrap();
        assert_eq!(a.admits().len(), 1, "κ_3 → κ_3 must invoke Drake-extended");
        assert_eq!(
            a.admits()[0].bridge,
            crate::diakrisis_bridge::BridgeId::DrakeReflectionExtended
        );
    }

    #[test]
    fn v2_admits_multi_step_ascent_via_drake_extended() {
        // κ_1 → κ_3 — multi-step (skips κ_2). admits via 131.L4.
        let mut a = BridgeAudit::new();
        check_universe_ascent_v2(
            KappaTier::KappaN(1),
            KappaTier::KappaN(3),
            &mut a,
            "κ_1→κ_3 multi-step",
        )
        .unwrap();
        assert_eq!(a.admits().len(), 1);
    }

    #[test]
    fn v2_rejects_tier_inversion() {
        // κ_2 → κ_1 — must reject.
        let mut a = BridgeAudit::new();
        let err = check_universe_ascent_v2(
            KappaTier::KappaN(2),
            KappaTier::KappaN(1),
            &mut a,
            "κ_2 → κ_1 inversion",
        )
        .unwrap_err();
        assert!(matches!(err, KernelError::UniverseAscentInvalid { .. }));
    }

    #[test]
    fn v2_rejects_higher_kappa_inversion() {
        // κ_5 → κ_3 — inversion at extended levels too.
        let mut a = BridgeAudit::new();
        let err = check_universe_ascent_v2(
            KappaTier::KappaN(5),
            KappaTier::KappaN(3),
            &mut a,
            "κ_5 → κ_3 inversion",
        )
        .unwrap_err();
        assert!(matches!(err, KernelError::UniverseAscentInvalid { .. }));
    }

    #[test]
    fn v2_rejects_truncated_to_kappa() {
        // Truncated → κ_1 is REJECTED (the user must lift first).
        let mut a = BridgeAudit::new();
        let err = check_universe_ascent_v2(
            KappaTier::Truncated,
            KappaTier::KappaN(1),
            &mut a,
            "Trunc → κ_1",
        )
        .unwrap_err();
        assert!(matches!(err, KernelError::UniverseAscentInvalid { .. }));
    }

    #[test]
    fn v2_rejects_kappa_to_truncated() {
        let mut a = BridgeAudit::new();
        let err = check_universe_ascent_v2(
            KappaTier::KappaN(1),
            KappaTier::Truncated,
            &mut a,
            "κ_1 → Trunc",
        )
        .unwrap_err();
        assert!(matches!(err, KernelError::UniverseAscentInvalid { .. }));
    }

    // ----- Ordinal bridge tests (V2 → native ordinal) -----

    #[test]
    fn kappa_tier_to_ordinal_truncated_is_finite_0() {
        use crate::ordinal::Ordinal;
        assert_eq!(KappaTier::Truncated.to_ordinal(), Ordinal::Finite(0));
    }

    #[test]
    fn kappa_tier_to_ordinal_kappa_n_is_kappa() {
        use crate::ordinal::Ordinal;
        assert_eq!(KappaTier::KappaN(1).to_ordinal(), Ordinal::Kappa(1));
        assert_eq!(KappaTier::KappaN(7).to_ordinal(), Ordinal::Kappa(7));
    }

    #[test]
    fn kappa_tier_from_ordinal_round_trip() {
        use crate::ordinal::Ordinal;
        // Truncated round-trip.
        let trunc = Ordinal::Finite(0);
        assert_eq!(KappaTier::from_ordinal(&trunc), Some(KappaTier::Truncated));
        // Kappa round-trips.
        for n in 1..=10 {
            let o = Ordinal::Kappa(n);
            assert_eq!(KappaTier::from_ordinal(&o), Some(KappaTier::KappaN(n)));
        }
        // Other ordinals: not representable in KappaTier.
        assert_eq!(KappaTier::from_ordinal(&Ordinal::Omega), None);
        assert_eq!(KappaTier::from_ordinal(&Ordinal::OmegaSquared), None);
        assert_eq!(KappaTier::from_ordinal(&Ordinal::Finite(1)), None);
    }

    #[test]
    fn kappa_tier_is_regular() {
        // Truncated is not regular (no inaccessible content).
        assert!(!KappaTier::Truncated.is_regular());
        // Every KappaN is regular (inaccessibles).
        assert!(KappaTier::KappaN(1).is_regular());
        assert!(KappaTier::KappaN(7).is_regular());
    }

    #[test]
    fn kappa_tier_is_inaccessible() {
        assert!(!KappaTier::Truncated.is_inaccessible());
        assert!(KappaTier::KappaN(1).is_inaccessible());
    }

    #[test]
    fn ordinal_bridge_lt_consistency() {
        // KappaTier::lt should agree with Ordinal::lt on the projected values.
        let pairs = vec![
            (KappaTier::Truncated, KappaTier::KappaN(1)),
            (KappaTier::KappaN(1), KappaTier::KappaN(2)),
            (KappaTier::KappaN(7), KappaTier::KappaN(99)),
        ];
        for (a, b) in &pairs {
            assert_eq!(
                a.lt(b),
                a.to_ordinal().lt(&b.to_ordinal()),
                "KappaTier::lt and Ordinal::lt disagree on ({:?}, {:?})",
                a,
                b
            );
        }
    }

    // ----------------------------------------------------------------
    // meta() consolidation drift pins for UniverseTier. The legacy
    // `lt` was a 6-arm match; `succ` was a 3-arm match with an
    // intentional saturation at κ_2 (Lemma 131.L3 / Theorem 134.T).
    // The consolidation makes both rules ride a single dense
    // `rank: u8` field; these pins close the structural rules so a
    // future variant addition can't silently break them.
    // ----------------------------------------------------------------

    #[test]
    fn meta_pin_universe_tier_round_trip_unique_and_dense_rank() {
        assert_eq!(UniverseTier::ALL.len(), 3);
        for v in UniverseTier::ALL {
            let s = v.as_str();
            assert_eq!(
                UniverseTier::from_str(s),
                Some(*v),
                "UniverseTier::{:?}: '{}' must round-trip",
                v,
                s
            );
        }
        // Names are unique.
        let names: Vec<&str> =
            UniverseTier::ALL.iter().map(|v| v.as_str()).collect();
        let mut dedup = names.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), names.len());
        // Ranks are dense 0..=2 in declaration order.
        for (i, v) in UniverseTier::ALL.iter().enumerate() {
            assert_eq!(
                v.rank() as usize,
                i,
                "UniverseTier::{:?}: rank drift at slot {}",
                v,
                i
            );
        }
        // Wire-form spot pins.
        assert_eq!(UniverseTier::Truncated.as_str(), "truncated");
        assert_eq!(UniverseTier::Kappa1.as_str(), "κ_1");
        assert_eq!(UniverseTier::Kappa2.as_str(), "κ_2");
        assert!(UniverseTier::from_str("κ_3").is_none());
    }

    #[test]
    fn meta_pin_universe_tier_lt_and_succ_invariants() {
        // Strict ordering: Truncated < κ_1 < κ_2.
        assert!(UniverseTier::Truncated.lt(&UniverseTier::Kappa1));
        assert!(UniverseTier::Truncated.lt(&UniverseTier::Kappa2));
        assert!(UniverseTier::Kappa1.lt(&UniverseTier::Kappa2));
        // Reflexive: nothing < itself.
        for v in UniverseTier::ALL {
            assert!(!v.lt(v), "UniverseTier::{:?}: lt must be irreflexive", v);
        }
        // Antisymmetric: a < b ⇒ ¬(b < a).
        for a in UniverseTier::ALL {
            for b in UniverseTier::ALL {
                if a.lt(b) {
                    assert!(!b.lt(a), "lt antisymmetry: {:?} < {:?}", a, b);
                }
            }
        }
        // Transitive: a < b ∧ b < c ⇒ a < c.
        for a in UniverseTier::ALL {
            for b in UniverseTier::ALL {
                for c in UniverseTier::ALL {
                    if a.lt(b) && b.lt(c) {
                        assert!(
                            a.lt(c),
                            "lt transitivity: {:?} < {:?} < {:?}",
                            a,
                            b,
                            c
                        );
                    }
                }
            }
        }

        // Successor: Truncated → κ_1 → κ_2 → κ_2 (saturates at top
        // per Lemma 131.L3 / Theorem 134.T tight 2-inacc bound).
        // **This is a load-bearing kernel rule** — the kernel admit
        // policy depends on it.
        assert_eq!(UniverseTier::Truncated.succ(), UniverseTier::Kappa1);
        assert_eq!(UniverseTier::Kappa1.succ(), UniverseTier::Kappa2);
        assert_eq!(
            UniverseTier::Kappa2.succ(),
            UniverseTier::Kappa2,
            "κ_2 must saturate per Drake-reflection closure"
        );
        // Cross-pin: succ never moves below the current tier.
        for v in UniverseTier::ALL {
            assert!(
                !v.succ().lt(v),
                "UniverseTier::{:?}: succ must not regress",
                v
            );
        }
        // Cross-pin: succ at non-top advances by exactly 1 in rank;
        // succ at top stays.
        for v in UniverseTier::ALL {
            let advance = (v.succ().rank() as i32) - (v.rank() as i32);
            let is_top = *v == UniverseTier::Kappa2;
            assert_eq!(
                advance,
                if is_top { 0 } else { 1 },
                "UniverseTier::{:?}: succ rank advance",
                v
            );
        }
    }
}
