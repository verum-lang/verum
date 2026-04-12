//! Epistemic state propagation encoding for SMT verification.
//!
//! In the quantum-epistemic extension of `core/math/linalg.vr`, an
//! `EpistemicState<N>` is a density matrix `ρ : Matrix<Complex, N, N>`
//! satisfying:
//!
//!   * `is_positive_semidefinite(ρ)`
//!   * `trace(ρ) = 1.0`
//!
//! Projective measurement and partial trace operations preserve
//! these invariants. This module encodes the constraint-satisfaction
//! problem for verifying that a composite operation preserves the
//! epistemic-state invariants.

use verum_common::{List, Text};

/// An epistemic-state invariant check.
#[derive(Debug, Clone)]
pub struct EpistemicInvariant {
    /// Dimension `N` of the density matrix.
    pub dim: usize,
    /// Does `ρ` satisfy positive semi-definiteness?
    pub is_psd: bool,
    /// Does `tr(ρ) = 1.0` hold?
    pub trace_normalized: bool,
}

impl EpistemicInvariant {
    pub fn new(dim: usize) -> Self {
        Self {
            dim,
            is_psd: false,
            trace_normalized: false,
        }
    }

    pub fn with_psd(mut self, psd: bool) -> Self {
        self.is_psd = psd;
        self
    }

    pub fn with_normalized_trace(mut self, normalized: bool) -> Self {
        self.trace_normalized = normalized;
        self
    }

    /// Is this a valid epistemic state?
    pub fn is_valid(&self) -> bool {
        self.dim > 0 && self.is_psd && self.trace_normalized
    }
}

/// Projective measurement: given an epistemic state and a projector,
/// verify that the post-measurement state remains a valid epistemic
/// state (Born rule + state collapse).
#[derive(Debug, Clone)]
pub struct ProjectiveMeasurement {
    pub pre_state: EpistemicInvariant,
    pub projector_dim: usize,
    pub outcome_probability_in_range: bool, // 0 ≤ p ≤ 1
}

impl ProjectiveMeasurement {
    pub fn new(pre_state: EpistemicInvariant, projector_dim: usize) -> Self {
        Self {
            pre_state,
            projector_dim,
            outcome_probability_in_range: true,
        }
    }

    /// Verify the measurement is well-formed: dimensions match and
    /// the pre-state is valid.
    pub fn is_well_formed(&self) -> bool {
        self.pre_state.is_valid()
            && self.projector_dim == self.pre_state.dim
            && self.outcome_probability_in_range
    }
}

/// Partial trace: project from `EpistemicState<N>` to `EpistemicState<M>`
/// where `M < N`, tracing out the complementary subsystem.
#[derive(Debug, Clone)]
pub struct PartialTrace {
    pub source_dim: usize,
    pub target_dim: usize,
    pub preserves_psd: bool,
    pub preserves_trace: bool,
}

impl PartialTrace {
    pub fn new(source_dim: usize, target_dim: usize) -> Self {
        Self {
            source_dim,
            target_dim,
            // By the CPTP-map theorem, partial trace preserves both
            // positive semi-definiteness and trace-normalization.
            preserves_psd: true,
            preserves_trace: true,
        }
    }

    pub fn is_valid_cptp_map(&self) -> bool {
        self.target_dim < self.source_dim
            && self.preserves_psd
            && self.preserves_trace
    }
}

/// Constraint-satisfaction result for epistemic-state propagation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpistemicResult {
    InvariantsPreserved,
    PsdViolated,
    TraceViolated,
    DimensionMismatch,
    Undetermined,
}

/// Check that an operation (measurement, partial trace, unitary
/// evolution) preserves epistemic-state invariants.
pub fn verify_invariants_preserved(
    pre: &EpistemicInvariant,
    post: &EpistemicInvariant,
) -> EpistemicResult {
    if pre.dim != post.dim && post.dim == 0 {
        return EpistemicResult::DimensionMismatch;
    }
    if !post.is_psd {
        return EpistemicResult::PsdViolated;
    }
    if !post.trace_normalized {
        return EpistemicResult::TraceViolated;
    }
    EpistemicResult::InvariantsPreserved
}

/// Concrete axiom set for epistemic propagation — used to seed the
/// SMT solver with the well-known facts that downstream verification
/// can rely on.
pub fn epistemic_axioms() -> List<Text> {
    List::from_iter([
        Text::from("∀ρ. is_psd(ρ) ∧ tr(ρ) = 1 → is_epistemic_state(ρ)"),
        Text::from("∀ρ Π. is_epistemic_state(ρ) ∧ is_projector(Π) → prob(Π, ρ) ∈ [0, 1]"),
        Text::from("∀ρ U. is_epistemic_state(ρ) ∧ is_unitary(U) → is_epistemic_state(U·ρ·U†)"),
        Text::from("∀ρ. is_epistemic_state(ρ) → tr(partial_trace(ρ)) = tr(ρ)"),
        Text::from("∀ρ. is_epistemic_state(ρ) → is_psd(partial_trace(ρ))"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_epistemic_state() {
        let inv = EpistemicInvariant::new(2)
            .with_psd(true)
            .with_normalized_trace(true);
        assert!(inv.is_valid());
    }

    #[test]
    fn test_invalid_psd() {
        let inv = EpistemicInvariant::new(2)
            .with_psd(false)
            .with_normalized_trace(true);
        assert!(!inv.is_valid());
    }

    #[test]
    fn test_invalid_trace() {
        let inv = EpistemicInvariant::new(2)
            .with_psd(true)
            .with_normalized_trace(false);
        assert!(!inv.is_valid());
    }

    #[test]
    fn test_projective_measurement_well_formed() {
        let pre = EpistemicInvariant::new(4)
            .with_psd(true)
            .with_normalized_trace(true);
        let meas = ProjectiveMeasurement::new(pre, 4);
        assert!(meas.is_well_formed());
    }

    #[test]
    fn test_projective_measurement_dim_mismatch() {
        let pre = EpistemicInvariant::new(4)
            .with_psd(true)
            .with_normalized_trace(true);
        let meas = ProjectiveMeasurement::new(pre, 3); // dim mismatch
        assert!(!meas.is_well_formed());
    }

    #[test]
    fn test_partial_trace_valid() {
        let pt = PartialTrace::new(4, 2);
        assert!(pt.is_valid_cptp_map());
    }

    #[test]
    fn test_partial_trace_invalid_target_larger() {
        let pt = PartialTrace::new(2, 4);
        assert!(!pt.is_valid_cptp_map());
    }

    #[test]
    fn test_invariants_preserved_good() {
        let pre = EpistemicInvariant::new(2).with_psd(true).with_normalized_trace(true);
        let post = EpistemicInvariant::new(2).with_psd(true).with_normalized_trace(true);
        assert_eq!(
            verify_invariants_preserved(&pre, &post),
            EpistemicResult::InvariantsPreserved
        );
    }

    #[test]
    fn test_invariants_preserved_psd_violated() {
        let pre = EpistemicInvariant::new(2).with_psd(true).with_normalized_trace(true);
        let post = EpistemicInvariant::new(2).with_psd(false).with_normalized_trace(true);
        assert_eq!(
            verify_invariants_preserved(&pre, &post),
            EpistemicResult::PsdViolated
        );
    }

    #[test]
    fn test_axioms_non_empty() {
        let axioms = epistemic_axioms();
        assert_eq!(axioms.len(), 5);
    }
}
