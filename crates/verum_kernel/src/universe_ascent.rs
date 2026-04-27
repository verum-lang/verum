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

impl UniverseTier {
    /// Canonical text rendering for diagnostic surfaces.
    pub fn as_str(&self) -> &'static str {
        match self {
            UniverseTier::Truncated => "truncated",
            UniverseTier::Kappa1    => "κ_1",
            UniverseTier::Kappa2    => "κ_2",
        }
    }

    /// Strict universe ordering: Truncated < κ_1 < κ_2.
    pub fn lt(&self, other: &Self) -> bool {
        match (self, other) {
            (UniverseTier::Truncated, UniverseTier::Kappa1)
            | (UniverseTier::Truncated, UniverseTier::Kappa2)
            | (UniverseTier::Kappa1,    UniverseTier::Kappa2) => true,
            _ => false,
        }
    }

    /// Successor: Truncated → κ_1 → κ_2 → κ_2 (saturates at the top
    /// per Lemma 131.L3 / Theorem 134.T tight-bound).
    pub fn succ(&self) -> Self {
        match self {
            UniverseTier::Truncated => UniverseTier::Kappa1,
            UniverseTier::Kappa1    => UniverseTier::Kappa2,
            UniverseTier::Kappa2    => UniverseTier::Kappa2,
        }
    }
}

/// Categorical coherence — `K-Universe-Ascent` kernel rule.
///
/// Verifies that a meta-classifier application `M_stack(α)`
/// correctly ascends the universe level by exactly one step:
///
/// ```text
///     Γ ⊢ α : Articulation@U_k       Γ ⊢ M_stack(α) : Articulation@U_{k+1}
///     ──────────────────────────────────────────────────────────────────── (K-Universe-Ascent)
///     Γ ⊢ M_stack : Functor[Articulation@U_k → Articulation@U_{k+1}]
/// ```
///
/// Per Lemma 131.L1 (universe-ascent): M_stack(F: U_1) ∈ U_2.
/// Per Lemma 131.L3 (Drake-reflection closure): M_stack(F: U_2)
/// stays in U_2; no κ_3 is needed.
///
/// The rule rejects:
///   - source/target tier inversion (target tier < source tier);
///   - source = Truncated with target ≥ Kappa1 — Truncated is the
///     Cat-baseline; meta-classifier application must start from
///     κ_1 or κ_2 per Theorem 131.T;
///   - source = Kappa2 with target = Kappa1 — would violate the
///     tight bound;
/// and accepts:
///   - source = κ_1, target = κ_2 (the canonical ascent);
///   - source = κ_2, target = κ_2 (Drake-reflection closure);
///   - source = Truncated, target = Truncated (Cat-baseline
///     identity, no ascent claimed).
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
