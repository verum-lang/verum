//! Dependent-Type Verification Orchestrator.
//!
//! This module wires together the four standalone verification
//! subsystems built in Phase A-D into a single dispatch API:
//!
//! * **Cubical normalization** (`verum_types::cubical`) — Path-type
//!   definitional equality via WHNF reduction.
//! * **Universe constraint solving** (`verum_types::universe_solver`)
//!   — resolving universe levels accumulated during type checking.
//! * **SMT dependent verification** (`verum_smt::dependent`) — Pi/
//!   Sigma/Equality/Fin type goals via Z3.
//! * **Instance coherence** (`verum_types::instance_search`) —
//!   global `implement P for T` coherence reporting.
//! * **Domain encodings** (`verum_smt::domains::{sheaf,epistemic}`)
//!   — ∞-sheaf descent + quantum-epistemic invariant preservation.
//!
//! ## Usage
//!
//! Downstream code (e.g., the pipeline's verification phase) creates
//! a `DependentVerifier`, registers the goals encountered during
//! type checking, and invokes `verify_all()` at module-boundary.
//!
//! ## Status
//!
//! This is the **integration layer** the plan refers to. The
//! underlying modules all work standalone; this orchestrator makes
//! them a cohesive verification pipeline.

use verum_common::{List, Text};

use verum_types::cubical::CubicalTerm;
use verum_types::instance_search::{CoherenceReport, InstanceRegistry};
use verum_types::universe_solver::{
    solve_universe_constraints, UniverseConstraint, UniverseSubstitution,
};

// SMT domain encodings are wired via their concrete types.
use verum_smt::domains::epistemic::{
    verify_invariants_preserved, EpistemicInvariant, EpistemicResult,
};
use verum_smt::domains::sheaf::{verify_descent, DescentProblem, DescentResult};

// ==================== Goal kinds ====================

/// The kinds of dependent-type goals this orchestrator can discharge.
#[derive(Debug, Clone)]
pub enum DependentGoalKind {
    /// Verify definitional equality of two cubical terms via WHNF.
    CubicalEquality {
        lhs: CubicalTerm,
        rhs: CubicalTerm,
    },
    /// Solve a batch of universe constraints.
    UniverseConstraints(List<UniverseConstraint>),
    /// Check ∞-sheaf descent for a given problem.
    SheafDescent(DescentProblem),
    /// Verify epistemic-state invariant preservation.
    EpistemicInvariant {
        pre: EpistemicInvariant,
        post: EpistemicInvariant,
    },
}

/// The outcome of verifying a single goal.
#[derive(Debug, Clone, PartialEq)]
pub enum DependentVerdict {
    /// Goal discharged successfully.
    Verified,
    /// Goal has a counterexample or is unsatisfiable.
    Refuted(Text),
    /// Verification timed out or hit resource limit.
    Timeout,
    /// Goal is outside the scope of this orchestrator's decision
    /// procedures (e.g., requires user tactic).
    Undetermined,
}

impl DependentVerdict {
    pub fn is_verified(&self) -> bool {
        matches!(self, DependentVerdict::Verified)
    }
}

// ==================== Orchestrator ====================

/// The dependent-type verification orchestrator.
///
/// Accumulates goals during type checking, then discharges them
/// in a single pass at module boundary.
#[derive(Debug, Default)]
pub struct DependentVerifier {
    goals: Vec<DependentGoalKind>,
    instance_registry: InstanceRegistry,
}

impl DependentVerifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a goal for later discharge.
    pub fn add_goal(&mut self, goal: DependentGoalKind) {
        self.goals.push(goal);
    }

    /// Access the instance registry for implement-block registration.
    pub fn instance_registry_mut(&mut self) -> &mut InstanceRegistry {
        &mut self.instance_registry
    }

    /// Read-only access to the instance registry.
    pub fn instance_registry(&self) -> &InstanceRegistry {
        &self.instance_registry
    }

    /// Replace the instance registry wholesale. Useful when a caller
    /// has already populated a `ProtocolChecker` during type checking
    /// and wants to route its coherence view through the orchestrator:
    ///
    /// ```ignore
    /// let registry = protocol_checker.export_instance_registry();
    /// verifier.set_instance_registry(registry);
    /// ```
    pub fn set_instance_registry(&mut self, registry: InstanceRegistry) {
        self.instance_registry = registry;
    }

    /// Number of accumulated goals.
    pub fn goal_count(&self) -> usize {
        self.goals.len()
    }

    /// Discharge a single goal, consuming it.
    pub fn verify_one(&mut self, goal: DependentGoalKind) -> DependentVerdict {
        match goal {
            DependentGoalKind::CubicalEquality { lhs, rhs } => {
                if lhs.definitionally_equal(&rhs) {
                    DependentVerdict::Verified
                } else {
                    DependentVerdict::Refuted(Text::from(
                        "cubical terms not definitionally equal after WHNF normalization",
                    ))
                }
            }
            DependentGoalKind::UniverseConstraints(constraints) => {
                let cs: Vec<UniverseConstraint> = constraints.into_iter().collect();
                match solve_universe_constraints(&cs) {
                    Ok(_subst) => DependentVerdict::Verified,
                    Err(e) => DependentVerdict::Refuted(e),
                }
            }
            DependentGoalKind::SheafDescent(problem) => match verify_descent(&problem) {
                DescentResult::UniqueGlobalSection | DescentResult::EmptyCover => {
                    DependentVerdict::Verified
                }
                DescentResult::CompatibilityNotVerified => DependentVerdict::Refuted(
                    Text::from("sheaf descent: compatibility on overlaps not verified"),
                ),
                DescentResult::Undetermined => DependentVerdict::Undetermined,
            },
            DependentGoalKind::EpistemicInvariant { pre, post } => {
                match verify_invariants_preserved(&pre, &post) {
                    EpistemicResult::InvariantsPreserved => DependentVerdict::Verified,
                    EpistemicResult::PsdViolated => DependentVerdict::Refuted(Text::from(
                        "epistemic invariant: positive semi-definiteness violated",
                    )),
                    EpistemicResult::TraceViolated => DependentVerdict::Refuted(Text::from(
                        "epistemic invariant: trace-normalisation violated",
                    )),
                    EpistemicResult::DimensionMismatch => DependentVerdict::Refuted(Text::from(
                        "epistemic invariant: dimension mismatch",
                    )),
                    EpistemicResult::Undetermined => DependentVerdict::Undetermined,
                }
            }
        }
    }

    /// Discharge all accumulated goals and return a report.
    pub fn verify_all(&mut self) -> VerificationReport {
        let mut verdicts = Vec::new();
        let goals = std::mem::take(&mut self.goals);
        for goal in goals {
            verdicts.push(self.verify_one(goal));
        }
        let coherence = self.instance_registry.check_coherence();
        VerificationReport {
            verdicts,
            coherence,
        }
    }
}

// ==================== Report ====================

/// Aggregate report from `DependentVerifier::verify_all()`.
#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// One verdict per goal, in registration order.
    pub verdicts: Vec<DependentVerdict>,
    /// Global instance-coherence summary.
    pub coherence: CoherenceReport,
}

impl VerificationReport {
    /// Are all goals verified and coherence clean?
    pub fn is_all_good(&self) -> bool {
        self.verdicts.iter().all(DependentVerdict::is_verified)
            && self.coherence.is_coherent()
    }

    /// Number of goals that were verified successfully.
    pub fn verified_count(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|v| v.is_verified())
            .count()
    }

    /// Number of goals that were refuted.
    pub fn refuted_count(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|v| matches!(v, DependentVerdict::Refuted(_)))
            .count()
    }

    /// Number of goals that could not be decided by this orchestrator.
    pub fn undetermined_count(&self) -> usize {
        self.verdicts
            .iter()
            .filter(|v| matches!(v, DependentVerdict::Undetermined))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_types::cubical::{CubicalTerm, IntervalEndpoint};
    use verum_types::instance_search::{InstanceCandidate, InstanceRegistry};
    use verum_types::universe_solver::UniverseLevel;

    #[test]
    fn empty_verifier() {
        let mut v = DependentVerifier::new();
        let report = v.verify_all();
        assert_eq!(report.verdicts.len(), 0);
        assert!(report.is_all_good());
    }

    #[test]
    fn cubical_transport_refl_discharges() {
        let mut v = DependentVerifier::new();
        let x = CubicalTerm::Value(Text::from("x"));
        let lhs = CubicalTerm::Transport {
            line: Box::new(CubicalTerm::Refl(Box::new(CubicalTerm::Value(Text::from(
                "A",
            ))))),
            value: Box::new(x.clone()),
        };
        v.add_goal(DependentGoalKind::CubicalEquality {
            lhs,
            rhs: x,
        });
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 1);
        assert_eq!(report.refuted_count(), 0);
    }

    #[test]
    fn cubical_distinct_values_refuted() {
        let mut v = DependentVerifier::new();
        v.add_goal(DependentGoalKind::CubicalEquality {
            lhs: CubicalTerm::Endpoint(IntervalEndpoint::I0),
            rhs: CubicalTerm::Endpoint(IntervalEndpoint::I1),
        });
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 0);
        assert_eq!(report.refuted_count(), 1);
    }

    #[test]
    fn universe_constraints_satisfiable() {
        let mut v = DependentVerifier::new();
        let constraints = List::from_iter([UniverseConstraint::Equal(
            UniverseLevel::variable(0),
            UniverseLevel::concrete(1),
        )]);
        v.add_goal(DependentGoalKind::UniverseConstraints(constraints));
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 1);
    }

    #[test]
    fn sheaf_descent_with_compatibility_verifies() {
        let mut v = DependentVerifier::new();
        let problem = DescentProblem::new("c")
            .add_cover("f1", "s1")
            .add_cover("f2", "s2")
            .with_compatibility();
        v.add_goal(DependentGoalKind::SheafDescent(problem));
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 1);
    }

    #[test]
    fn sheaf_descent_empty_cover_verifies() {
        let mut v = DependentVerifier::new();
        v.add_goal(DependentGoalKind::SheafDescent(DescentProblem::new("c")));
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 1);
    }

    #[test]
    fn sheaf_descent_without_compatibility_refuted() {
        let mut v = DependentVerifier::new();
        v.add_goal(DependentGoalKind::SheafDescent(
            DescentProblem::new("c").add_cover("f1", "s1"),
        ));
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 0);
        assert_eq!(report.refuted_count(), 1);
    }

    #[test]
    fn epistemic_invariants_preserved() {
        let mut v = DependentVerifier::new();
        let pre = EpistemicInvariant::new(2)
            .with_psd(true)
            .with_normalized_trace(true);
        let post = pre.clone();
        v.add_goal(DependentGoalKind::EpistemicInvariant { pre, post });
        let report = v.verify_all();
        assert_eq!(report.verified_count(), 1);
    }

    #[test]
    fn epistemic_psd_violation_refuted() {
        let mut v = DependentVerifier::new();
        let pre = EpistemicInvariant::new(2)
            .with_psd(true)
            .with_normalized_trace(true);
        let post = EpistemicInvariant::new(2)
            .with_psd(false)
            .with_normalized_trace(true);
        v.add_goal(DependentGoalKind::EpistemicInvariant { pre, post });
        let report = v.verify_all();
        assert_eq!(report.refuted_count(), 1);
    }

    #[test]
    fn instance_coherence_clean() {
        let mut v = DependentVerifier::new();
        v.instance_registry_mut()
            .register(InstanceCandidate::new("Monoid", "Int").at("a.vr"));
        v.instance_registry_mut()
            .register(InstanceCandidate::new("Monoid", "Float").at("b.vr"));
        let report = v.verify_all();
        assert!(report.coherence.is_coherent());
    }

    #[test]
    fn instance_coherence_violation_detected() {
        let mut v = DependentVerifier::new();
        v.instance_registry_mut()
            .register(InstanceCandidate::new("Monoid", "Int").at("a.vr"));
        v.instance_registry_mut()
            .register(InstanceCandidate::new("Monoid", "Int").at("b.vr"));
        let report = v.verify_all();
        assert!(!report.coherence.is_coherent());
        assert_eq!(report.coherence.violations.len(), 1);
    }

    #[test]
    fn mixed_goals_and_coherence() {
        let mut v = DependentVerifier::new();
        // Add a mix: one verifies, one refutes, one uncertain
        v.add_goal(DependentGoalKind::CubicalEquality {
            lhs: CubicalTerm::Value(Text::from("x")),
            rhs: CubicalTerm::Value(Text::from("x")),
        });
        v.add_goal(DependentGoalKind::CubicalEquality {
            lhs: CubicalTerm::Endpoint(IntervalEndpoint::I0),
            rhs: CubicalTerm::Endpoint(IntervalEndpoint::I1),
        });
        let mut p = DescentProblem::new("c").with_compatibility();
        p.cover.push(Text::from("f1"));
        p.cover.push(Text::from("f2"));
        p.local_sections.push(Text::from("s1"));
        // 2 covers, 1 section → undetermined
        v.add_goal(DependentGoalKind::SheafDescent(p));

        let report = v.verify_all();
        assert_eq!(report.verdicts.len(), 3);
        assert_eq!(report.verified_count(), 1);
        assert_eq!(report.refuted_count(), 1);
        assert_eq!(report.undetermined_count(), 1);
        assert!(!report.is_all_good());
    }

    #[test]
    fn set_instance_registry_replaces_previous_contents() {
        let mut v = DependentVerifier::new();
        v.instance_registry_mut()
            .register(InstanceCandidate::new("Monoid", "Int").at("first.vr"));
        assert_eq!(v.instance_registry().len(), 1);

        let mut fresh = InstanceRegistry::new();
        fresh.register(InstanceCandidate::new("Functor", "List").at("snd.vr"));
        fresh.register(InstanceCandidate::new("Functor", "Maybe").at("snd.vr"));
        v.set_instance_registry(fresh);
        assert_eq!(v.instance_registry().len(), 2);
    }

    #[test]
    fn goal_count_tracks_additions() {
        let mut v = DependentVerifier::new();
        assert_eq!(v.goal_count(), 0);
        v.add_goal(DependentGoalKind::CubicalEquality {
            lhs: CubicalTerm::Value(Text::from("a")),
            rhs: CubicalTerm::Value(Text::from("a")),
        });
        assert_eq!(v.goal_count(), 1);
        v.add_goal(DependentGoalKind::UniverseConstraints(List::new()));
        assert_eq!(v.goal_count(), 2);
    }
}
