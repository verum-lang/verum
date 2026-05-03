//! Modal-Temporal Architectural Calculus (MTAC) — primitives.
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §20-§23 (fundamental rethinking of
//! architectural types), classical static notations like C4/UML
//! treat architecture as a **point** in shape-space. Real
//! architecture is a **functor** from time × decisions × observers
//! to shape:
//!
//! `arch_type<C>: TimeCategory × DecisionCategory × ObserverCategory → ShapeCategory`
//!
//! MTAC primitives establish the categories that this functor maps
//! between. ships them as data types; + wires them
//! into anti-pattern checks (TemporalInconsistency,
//! CounterfactualBrittleness, etc. — AP-027..032 already in the
//! catalog).
//!
//! ## Why Verum, not Coq/Lean
//!
//! No production proof assistant treats architecture as a
//! functor over time + decisions + observers. Coq/Lean stop at
//! single-shape types. Verum's MTAC is the first attempt to
//! make modal-temporal reasoning about architecture compile-time
//! enforceable per spec §32.

use serde::{Deserialize, Serialize};

// =============================================================================
// TimePoint — point in the time category
// =============================================================================

/// A point in the time category. Per spec §20.1, time is a
/// non-linear lattice (branching futures, counterfactual past
/// branches), not a strict linear order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimePoint {
 /// Past — concrete historical timestamp.
    Past(i64), // unix-epoch seconds
 /// Now — the current moment.
    Now,
 /// Future — projected timestamp (target date).
    Future(i64),
    /// Counterfactual — alternative-history branch.
    Counterfactual {
        /// Identifier naming the counterfactual branch.
        branch: String,
    },
}

impl TimePoint {
 /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            TimePoint::Past(_) => "past",
            TimePoint::Now => "now",
            TimePoint::Future(_) => "future",
            TimePoint::Counterfactual { .. } => "counterfactual",
        }
    }

 /// Chronological partial order: `self ≤ other`. Returns
 /// `false` when the two points are not comparable (different
 /// counterfactual branches).
    pub fn precedes(&self, other: &TimePoint) -> bool {
        match (self, other) {
            (TimePoint::Past(a), TimePoint::Past(b)) => a <= b,
            (TimePoint::Past(_), TimePoint::Now) => true,
            (TimePoint::Past(_), TimePoint::Future(_)) => true,
            (TimePoint::Now, TimePoint::Now) => true,
            (TimePoint::Now, TimePoint::Future(_)) => true,
            (TimePoint::Future(a), TimePoint::Future(b)) => a <= b,
 // Counterfactual branches are never directly comparable.
            (TimePoint::Counterfactual { branch: a }, TimePoint::Counterfactual { branch: b }) => {
                a == b
            }
            _ => false,
        }
    }
}

// =============================================================================
// Decision — point in the decision category
// =============================================================================

/// An architectural decision — selects one of several options.
/// Per spec §20.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
 /// Stable name (e.g. "framework_choice").
    pub name: String,
 /// Possible values for this decision.
    pub options: Vec<DecisionOption>,
 /// Currently-chosen value, if fixed.
    pub chosen: Option<DecisionOption>,
 /// Decisions this one depends on.
    pub depends_on: Vec<String>,
}

/// One option within a [`Decision`] — a candidate value the
/// architectural decision can resolve to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionOption {
    /// Stable identifier for this option.
    pub name: String,
    /// Human-readable explanation of the option.
    pub description: String,
}

impl Decision {
 /// True iff `self` is a refinement of `other` — self's chosen
 /// option is one of other's options.
    pub fn refines(&self, other: &Decision) -> bool {
        self.name == other.name
            && match (&self.chosen, &other.chosen) {
                (Some(a), Some(b)) => a == b,
                (Some(a), None) => other.options.contains(a),
                _ => false,
            }
    }

 /// True iff a decision is fully resolved (chosen value set).
    pub fn is_resolved(&self) -> bool {
        self.chosen.is_some()
    }
}

// =============================================================================
// Observer — point in the observer category
// =============================================================================

/// An observer of the system — Yoneda-driven design per spec §20.1.
/// Architecture is uniquely characterised by what it does to all
/// observers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Observer {
    /// Generic end-user.
    EndUser {
        /// End-user category (default / power / admin / ...).
        kind: String,
    },
    /// Another cog in the composition graph.
    PeerCog {
        /// Dotted module path of the peer cog.
        module_path: String,
    },
    /// Stakeholder with a special role.
    Stakeholder {
        /// Stakeholder role (operator / regulator / customer / ...).
        role: String,
    },
    /// Auditor — verifying compliance.
    Auditor {
        /// Audit kind (compliance / security / financial / ...).
        audit_kind: String,
    },
    /// Adversary — threat-model observer.
    Adversary {
        /// Threat model the adversary operates under.
        threat_model: String,
    },
}

impl Observer {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            Observer::EndUser { .. } => "end_user",
            Observer::PeerCog { .. } => "peer_cog",
            Observer::Stakeholder { .. } => "stakeholder",
            Observer::Auditor { .. } => "auditor",
            Observer::Adversary { .. } => "adversary",
        }
    }

 /// Full canonical observer roster.
    pub fn full_canonical_roster() -> Vec<Observer> {
        vec![
            Observer::EndUser {
                kind: "default".into(),
            },
            Observer::PeerCog {
                module_path: "<any>".into(),
            },
            Observer::Stakeholder {
                role: "operator".into(),
            },
            Observer::Auditor {
                audit_kind: "compliance".into(),
            },
            Observer::Adversary {
                threat_model: "external".into(),
            },
        ]
    }
}

// =============================================================================
// ModalAssertion — modal logic over architecture (MAL)
// =============================================================================

/// Modal Architectural Logic (MAL) operators per spec §20.3.
/// Combines S4/S5-style modal logic с LTL temporal operators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModalAssertion {
    /// `□ A` — A holds in EVERY possible future / decision branch.
    Necessity {
        /// Proposition that must hold in every branch.
        proposition: ArchProposition,
    },
    /// `◇ A` — A holds in SOME possible future / decision branch.
    Possibility {
        /// Proposition that holds in at least one branch.
        proposition: ArchProposition,
    },
    /// `◇F A` — A holds in some future time-point.
    Eventually {
        /// Proposition that eventually holds.
        proposition: ArchProposition,
    },
    /// `□G A` — A holds in every future time-point.
    Always {
        /// Proposition that holds at every future time-point.
        proposition: ArchProposition,
    },
    /// `A U B` — A holds until B holds.
    Until {
        /// Proposition `A` that must hold until `B` arrives.
        first: ArchProposition,
        /// Proposition `B` that ends the `Until` window.
        second: ArchProposition,
    },
    /// `A ⇨ B` — counterfactual: if A held, B would hold.
    Counterfactual {
        /// Counterfactual antecedent (`A`).
        antecedent: ArchProposition,
        /// Counterfactual consequent (`B`).
        consequent: ArchProposition,
    },
}

impl ModalAssertion {
 /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            ModalAssertion::Necessity { .. } => "necessity",
            ModalAssertion::Possibility { .. } => "possibility",
            ModalAssertion::Eventually { .. } => "eventually",
            ModalAssertion::Always { .. } => "always",
            ModalAssertion::Until { .. } => "until",
            ModalAssertion::Counterfactual { .. } => "counterfactual",
        }
    }

 /// True iff this is a temporal operator (Eventually / Always / Until).
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            ModalAssertion::Eventually { .. }
                | ModalAssertion::Always { .. }
                | ModalAssertion::Until { .. }
        )
    }

 /// True iff this is a modal operator (Necessity / Possibility).
    pub fn is_modal(&self) -> bool {
        matches!(
            self,
            ModalAssertion::Necessity { .. } | ModalAssertion::Possibility { .. }
        )
    }
}

// =============================================================================
// ArchProposition — content of a modal assertion
// =============================================================================

/// An architectural proposition — what a modal operator quantifies
/// over. baseline: capability presence + invariant
/// preservation + foundation stability. extends with
/// arbitrary refinement predicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchProposition {
    /// Capability of given name is present in the cog's exposes/requires.
    HasCapability {
        /// Capability tag (per `Capability::tag`).
        capability_tag: String,
    },
    /// Foundation remains stable across time/decisions.
    FoundationStable,
    /// API is unchanged (public interface invariant).
    PublicApiUnchanged,
    /// Custom predicate referenced by name.
    Custom {
        /// Refinement-predicate name resolved at audit time.
        name: String,
    },
}

impl ArchProposition {
    /// Stable diagnostic tag used in audit JSON + ATS-V error codes.
    pub fn tag(&self) -> &'static str {
        match self {
            ArchProposition::HasCapability { .. } => "has_capability",
            ArchProposition::FoundationStable => "foundation_stable",
            ArchProposition::PublicApiUnchanged => "public_api_unchanged",
            ArchProposition::Custom { .. } => "custom",
        }
    }
}

// =============================================================================
// ArchEvolution — future-oriented type
// =============================================================================

/// An expected evolution of the cog's architecture per spec §21.3.
/// Trigger conditions, target shape change, complexity class,
/// reversibility — all type-level.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchEvolution {
    /// Trigger condition (e.g. "capability_X_becomes_available").
    pub trigger: String,
    /// Target time-point when the evolution is expected.
    pub expected_time: TimePoint,
    /// Cost class of the evolution.
    pub cost_class: ComplexityClass,
    /// Whether the evolution is reversible (adjoint pair exists).
    pub reversibility: Reversibility,
}

/// Asymptotic cost class of an [`ArchEvolution`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ComplexityClass {
    /// O(1) — trivial change (config flip).
    Trivial,
    /// O(N) — small change touching N modules.
    Linear,
    /// O(N²) — quadratic.
    Quadratic,
    /// Architectural redesign — bounded but expensive.
    ArchitecturalRedesign,
    /// Unbounded — full system rewrite.
    Rewrite,
}

/// Reversibility of an [`ArchEvolution`] under refactoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reversibility {
    /// Reversible via adjoint pair (left / right adjoint exists).
    AdjointReversible,
    /// One-way — cannot undo.
    Irreversible,
    /// Reversible up to a bound (limited rollback window).
    BoundedReversible {
        /// Rollback window measured in seconds.
        window_seconds: u64,
    },
}

// =============================================================================
// CounterfactualPair — what-if pair for spec §22 reasoning
// =============================================================================

/// A pair of (base, alternative) decisions used in counterfactual
/// evaluation per spec §22.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterfactualPair {
    /// Stable identifier for the counterfactual pair.
    pub name: String,
    /// The base / actual decision.
    pub base: Decision,
    /// The alternative / counterfactual decision.
    pub alternative: Decision,
    /// Propositions that must remain stable under the swap.
    pub stability_invariants: Vec<ArchProposition>,
}

// =============================================================================
// AdjunctionWitness — refactoring-as-adjunction per spec §20.6
// =============================================================================

/// A pair of functors representing a refactoring adjunction.
/// Per spec §20.6: every refactoring has a direction `F: Old → New`
/// and an adjoint `G: New → Old` with `F ⊣ G`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdjunctionWitness {
    /// Name of the forward functor `F`.
    pub forward_name: String,
    /// Name of the backward functor `G`.
    pub backward_name: String,
    /// Properties preserved under F.
    pub preserved: Vec<ArchProposition>,
    /// Properties gained under F.
    pub gained: Vec<ArchProposition>,
}

impl AdjunctionWitness {
 /// True iff this witness is a left-adjoint of `other`'s right-adjoint.
 /// simplification: structural equality of forward names.
    pub fn is_adjoint_of(&self, other: &AdjunctionWitness) -> bool {
        self.forward_name == other.backward_name && self.backward_name == other.forward_name
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_point_tags_distinct() {
        let probes = [
            TimePoint::Past(0),
            TimePoint::Now,
            TimePoint::Future(1000),
            TimePoint::Counterfactual {
                branch: "alt".into(),
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|t| t.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn time_precedes_chronological() {
 // Past(0) < Now < Future(1000)
        assert!(TimePoint::Past(0).precedes(&TimePoint::Past(100)));
        assert!(TimePoint::Past(0).precedes(&TimePoint::Now));
        assert!(TimePoint::Now.precedes(&TimePoint::Future(1000)));
        assert!(TimePoint::Future(1000).precedes(&TimePoint::Future(2000)));
    }

    #[test]
    fn time_counterfactual_branches_only_self_comparable() {
        let alt_a = TimePoint::Counterfactual {
            branch: "a".into(),
        };
        let alt_b = TimePoint::Counterfactual {
            branch: "b".into(),
        };
 // Same branch — comparable.
        assert!(alt_a.precedes(&alt_a.clone()));
 // Different branches — NOT comparable.
        assert!(!alt_a.precedes(&alt_b));
        assert!(!alt_b.precedes(&alt_a));
    }

    #[test]
    fn decision_resolved_only_when_chosen() {
        let unresolved = Decision {
            name: "framework".into(),
            options: vec![DecisionOption {
                name: "Vue".into(),
                description: "x".into(),
            }],
            chosen: None,
            depends_on: vec![],
        };
        assert!(!unresolved.is_resolved());

        let mut resolved = unresolved.clone();
        resolved.chosen = Some(DecisionOption {
            name: "Vue".into(),
            description: "x".into(),
        });
        assert!(resolved.is_resolved());
    }

    #[test]
    fn observer_canonical_roster_has_5() {
        let roster = Observer::full_canonical_roster();
        assert_eq!(roster.len(), 5);
        let tags: std::collections::BTreeSet<_> = roster.iter().map(|o| o.tag()).collect();
        assert_eq!(tags.len(), 5);
    }

    #[test]
    fn modal_assertion_tags_distinct() {
        let probes = [
            ModalAssertion::Necessity {
                proposition: ArchProposition::FoundationStable,
            },
            ModalAssertion::Possibility {
                proposition: ArchProposition::FoundationStable,
            },
            ModalAssertion::Eventually {
                proposition: ArchProposition::FoundationStable,
            },
            ModalAssertion::Always {
                proposition: ArchProposition::FoundationStable,
            },
            ModalAssertion::Until {
                first: ArchProposition::FoundationStable,
                second: ArchProposition::FoundationStable,
            },
            ModalAssertion::Counterfactual {
                antecedent: ArchProposition::FoundationStable,
                consequent: ArchProposition::FoundationStable,
            },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|m| m.tag()).collect();
        assert_eq!(tags.len(), 6);
    }

    #[test]
    fn modal_assertion_classifies_temporal_vs_modal() {
        let necessity = ModalAssertion::Necessity {
            proposition: ArchProposition::FoundationStable,
        };
        let always = ModalAssertion::Always {
            proposition: ArchProposition::FoundationStable,
        };
        assert!(necessity.is_modal());
        assert!(!necessity.is_temporal());
        assert!(always.is_temporal());
        assert!(!always.is_modal());
    }

    #[test]
    fn arch_proposition_tags_distinct() {
        let probes = [
            ArchProposition::HasCapability {
                capability_tag: "x".into(),
            },
            ArchProposition::FoundationStable,
            ArchProposition::PublicApiUnchanged,
            ArchProposition::Custom { name: "x".into() },
        ];
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|p| p.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn adjunction_witness_recognises_adjoint_pair() {
        let f = AdjunctionWitness {
            forward_name: "inline".into(),
            backward_name: "extract".into(),
            preserved: vec![],
            gained: vec![],
        };
        let g = AdjunctionWitness {
            forward_name: "extract".into(),
            backward_name: "inline".into(),
            preserved: vec![],
            gained: vec![],
        };
        assert!(f.is_adjoint_of(&g));
        assert!(g.is_adjoint_of(&f));
    }

    #[test]
    fn complexity_class_serde_roundtrip() {
        let c = ComplexityClass::ArchitecturalRedesign;
        let json = serde_json::to_string(&c).unwrap();
        let back: ComplexityClass = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn architectural_pin_mtac_primitive_count() {
 // Pin: ships exactly these MTAC primitives.
 // Adding more requires RFC ATS-V-007 (per spec §29.2).
 // - TimePoint: 4 variants
 // - Observer: 5 canonical
 // - ModalAssertion: 6 operators (Necessity/Possibility/Eventually/Always/Until/Counterfactual)
 // - ArchProposition: 4 baseline (HasCapability/FoundationStable/PublicApiUnchanged/Custom)
 // - ComplexityClass: 5 levels
 // - Reversibility: 3 kinds
        assert_eq!(Observer::full_canonical_roster().len(), 5);
 // 6 ModalAssertion operators verified by tags-distinct test above.
    }
}
