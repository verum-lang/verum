//! ATS-V Сезон 8 — Yoneda-equivalence checker.
//!
//! Per spec §20.7 + §23: two architectures are equivalent iff they
//! produce the same observation for every [`Observer`] in the
//! canonical roster.  Architecturally, this realises the Yoneda
//! lemma — an object `X` of category `C` is uniquely determined by
//! its representable functor `Hom(-, X) : C^op → Set`.  In ATS-V
//! terms: a cog is uniquely determined by how every observer sees
//! it, so two cogs are equivalent iff every observer's projection
//! agrees.
//!
//! # Pipeline
//!
//! 1. Caller supplies (`base_shape`, `alt_shape`, `observers`).
//!    Empty observers → uses [`Observer::full_canonical_roster`].
//! 2. [`observe`] projects each Shape from each Observer's
//!    viewpoint to a [`ShapeObservation`] — a typed subset of the
//!    Shape's fields the observer is sensitive to.
//! 3. Per-observer agreement is checked by structural equality of
//!    the two observations.
//! 4. [`yoneda_equivalent`] returns a [`YonedaVerdict`] with per-
//!    observer agreement + the aggregate verdict (equivalent iff
//!    every observer agrees).
//!
//! Per spec §20.7, ATS-V accepts a refactoring between
//! Yoneda-equivalent formulations as **trivially safe**.

use serde::{Deserialize, Serialize};

use crate::arch::{Capability, CveClosure, Foundation, Lifecycle, MsfsStratum, Shape, Tier};
use crate::arch_mtac::Observer;

// =============================================================================
// ShapeObservation — what an observer sees
// =============================================================================

/// Projection of a [`Shape`] from a single [`Observer`]'s viewpoint.
/// Variants are per-observer-kind so each carries only the fields
/// that observer is sensitive to.  Equality of two observations
/// implies the observer cannot distinguish the two underlying
/// shapes (Yoneda-relevant agreement).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "observer_kind")]
pub enum ShapeObservation {
    /// EndUser sees the public interface: exposes + lifecycle.
    EndUser {
        kind: String,
        exposes: Vec<Capability>,
        lifecycle: Lifecycle,
    },
    /// PeerCog sees what it composes against: composes_with
    /// containing the cog + the boundary capabilities (requires +
    /// exposes).
    PeerCog {
        module_path: String,
        is_in_composition: bool,
        requires: Vec<Capability>,
        exposes: Vec<Capability>,
    },
    /// Stakeholder sees deployment-level concerns: tier, foundation,
    /// persistence-related capabilities, lifecycle stage.
    Stakeholder {
        role: String,
        tier: Tier,
        foundation: Foundation,
        lifecycle: Lifecycle,
        persistence_capabilities: Vec<Capability>,
    },
    /// Auditor sees everything: full Shape projection.  This is the
    /// strictest observer — two shapes agree under Auditor iff
    /// their full Shape projections are identical.
    Auditor {
        audit_kind: String,
        exposes: Vec<Capability>,
        requires: Vec<Capability>,
        foundation: Foundation,
        stratum: MsfsStratum,
        cve_closure: CveClosure,
        tier: Tier,
        lifecycle: Lifecycle,
        composes_with: Vec<String>,
        strict: bool,
    },
    /// Adversary sees the attack surface: exposes + boundary
    /// invariants + capability handoffs (which capabilities cross
    /// the boundary outward).
    Adversary {
        threat_model: String,
        attack_surface: Vec<Capability>,
        outbound_capabilities: Vec<Capability>,
    },
}

impl ShapeObservation {
    /// Stable single-token observer-kind tag for JSON / agent
    /// surfaces.
    pub fn observer_tag(&self) -> &'static str {
        match self {
            ShapeObservation::EndUser { .. } => "end_user",
            ShapeObservation::PeerCog { .. } => "peer_cog",
            ShapeObservation::Stakeholder { .. } => "stakeholder",
            ShapeObservation::Auditor { .. } => "auditor",
            ShapeObservation::Adversary { .. } => "adversary",
        }
    }
}

// =============================================================================
// Per-observer projection
// =============================================================================

/// Project a [`Shape`] from a single [`Observer`]'s viewpoint to a
/// [`ShapeObservation`].  This is the core Yoneda operation —
/// `Hom(-, shape)(observer) = observation`.
pub fn observe(shape: &Shape, observer: &Observer) -> ShapeObservation {
    match observer {
        Observer::EndUser { kind } => ShapeObservation::EndUser {
            kind: kind.clone(),
            exposes: shape.exposes.clone(),
            lifecycle: shape.lifecycle.clone(),
        },
        Observer::PeerCog { module_path } => ShapeObservation::PeerCog {
            module_path: module_path.clone(),
            is_in_composition: module_path == "<any>"
                || shape.composes_with.iter().any(|c| c == module_path),
            requires: shape.requires.clone(),
            exposes: shape.exposes.clone(),
        },
        Observer::Stakeholder { role } => ShapeObservation::Stakeholder {
            role: role.clone(),
            tier: shape.at_tier.clone(),
            foundation: shape.foundation.clone(),
            lifecycle: shape.lifecycle.clone(),
            persistence_capabilities: shape
                .exposes
                .iter()
                .chain(shape.requires.iter())
                .filter(|c| matches!(c, Capability::Persist { .. }))
                .cloned()
                .collect(),
        },
        Observer::Auditor { audit_kind } => ShapeObservation::Auditor {
            audit_kind: audit_kind.clone(),
            exposes: shape.exposes.clone(),
            requires: shape.requires.clone(),
            foundation: shape.foundation.clone(),
            stratum: shape.stratum,
            cve_closure: shape.cve_closure.clone(),
            tier: shape.at_tier.clone(),
            lifecycle: shape.lifecycle.clone(),
            composes_with: shape.composes_with.clone(),
            strict: shape.strict,
        },
        Observer::Adversary { threat_model } => ShapeObservation::Adversary {
            threat_model: threat_model.clone(),
            attack_surface: shape.exposes.clone(),
            outbound_capabilities: shape
                .requires
                .iter()
                .filter(|c| {
                    matches!(
                        c,
                        Capability::Network { .. } | Capability::Exec { .. }
                    )
                })
                .cloned()
                .collect(),
        },
    }
}

// =============================================================================
// YonedaVerdict — equivalence outcome
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgreementStatus {
    /// Both observations are structurally identical.
    Agree,
    /// Observations differ — the observer can distinguish the
    /// shapes.
    Disagree,
}

impl AgreementStatus {
    pub fn tag(&self) -> &'static str {
        match self {
            AgreementStatus::Agree => "agree",
            AgreementStatus::Disagree => "disagree",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObserverAgreement {
    pub observer: Observer,
    pub status: AgreementStatus,
    /// Base-shape observation (for diagnostics).
    pub base_observation: ShapeObservation,
    /// Alt-shape observation (for diagnostics).
    pub alt_observation: ShapeObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YonedaVerdict {
    /// Stable JSON schema version (per spec §32.4).
    pub schema_version: u32,
    /// Per-observer agreement.
    pub agreements: Vec<ObserverAgreement>,
    /// Aggregate verdict — equivalent iff every observer agrees.
    pub equivalent: bool,
    /// Number of observers that disagreed.
    pub disagreement_count: usize,
}

// =============================================================================
// Engine entry points
// =============================================================================

/// Decide Yoneda-equivalence between two shapes against an observer
/// roster.  Empty roster → use the canonical 5-element roster
/// (`Observer::full_canonical_roster()`).
pub fn yoneda_equivalent(
    base_shape: &Shape,
    alt_shape: &Shape,
    observers: &[Observer],
) -> YonedaVerdict {
    let observers: Vec<Observer> = if observers.is_empty() {
        Observer::full_canonical_roster()
    } else {
        observers.to_vec()
    };

    let agreements: Vec<ObserverAgreement> = observers
        .iter()
        .map(|o| {
            let base_obs = observe(base_shape, o);
            let alt_obs = observe(alt_shape, o);
            let status = if base_obs == alt_obs {
                AgreementStatus::Agree
            } else {
                AgreementStatus::Disagree
            };
            ObserverAgreement {
                observer: o.clone(),
                status,
                base_observation: base_obs,
                alt_observation: alt_obs,
            }
        })
        .collect();

    let disagreement_count = agreements
        .iter()
        .filter(|a| a.status == AgreementStatus::Disagree)
        .count();
    let equivalent = disagreement_count == 0 && !agreements.is_empty();

    YonedaVerdict {
        schema_version: 1,
        agreements,
        equivalent,
        disagreement_count,
    }
}

/// Returns the list of observers under which the two shapes
/// disagree.  Empty list ⇔ Yoneda-equivalent.
pub fn distinguishing_observers(
    base_shape: &Shape,
    alt_shape: &Shape,
    observers: &[Observer],
) -> Vec<Observer> {
    yoneda_equivalent(base_shape, alt_shape, observers)
        .agreements
        .into_iter()
        .filter(|a| a.status == AgreementStatus::Disagree)
        .map(|a| a.observer)
        .collect()
}

/// Per spec §20.7: a refactoring is **trivially safe** iff its
/// before/after shapes are Yoneda-equivalent.  Direct convenience
/// wrapper for refactoring callsites.
pub fn refactoring_is_trivially_safe(
    before: &Shape,
    after: &Shape,
    observers: &[Observer],
) -> bool {
    yoneda_equivalent(before, after, observers).equivalent
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::{Foundation, NetDirection, NetProtocol, ResourceTag};

    fn end_user() -> Observer {
        Observer::EndUser {
            kind: "default".into(),
        }
    }
    fn auditor() -> Observer {
        Observer::Auditor {
            audit_kind: "compliance".into(),
        }
    }
    fn adversary() -> Observer {
        Observer::Adversary {
            threat_model: "external".into(),
        }
    }
    fn stakeholder() -> Observer {
        Observer::Stakeholder {
            role: "operator".into(),
        }
    }
    fn peer_cog(path: &str) -> Observer {
        Observer::PeerCog {
            module_path: path.into(),
        }
    }

    #[test]
    fn identity_shape_is_yoneda_equivalent_to_itself() {
        let s = Shape::default_for_unannotated();
        let v = yoneda_equivalent(&s, &s, &[]);
        assert!(v.equivalent);
        assert_eq!(v.disagreement_count, 0);
        assert_eq!(v.agreements.len(), 5); // canonical roster
    }

    #[test]
    fn empty_roster_with_empty_alt_means_unknown() {
        // When observers list is empty AND we pass through the
        // canonical-roster default, the verdict is computed against
        // the canonical 5.  Pin: no possibility of "0 observers,
        // 0 disagreements → equivalent" gaming.
        let s = Shape::default_for_unannotated();
        let v = yoneda_equivalent(&s, &s, &[]);
        assert_eq!(v.agreements.len(), 5);
    }

    #[test]
    fn auditor_sees_foundation_drift() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
        let v = yoneda_equivalent(&s_base, &s_alt, &[auditor()]);
        assert!(!v.equivalent);
        assert_eq!(v.disagreement_count, 1);
    }

    #[test]
    fn end_user_blind_to_foundation_drift() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
        let v = yoneda_equivalent(&s_base, &s_alt, &[end_user()]);
        // EndUser does not project foundation → cannot distinguish.
        assert!(v.equivalent);
    }

    #[test]
    fn end_user_sees_exposes_change() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let v = yoneda_equivalent(&s_base, &s_alt, &[end_user()]);
        assert!(!v.equivalent);
    }

    #[test]
    fn adversary_sees_outbound_network_capability() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.requires = vec![Capability::Network {
            protocol: NetProtocol::Tcp,
            direction: NetDirection::Outbound,
        }];
        let v = yoneda_equivalent(&s_base, &s_alt, &[adversary()]);
        assert!(!v.equivalent);
    }

    #[test]
    fn adversary_blind_to_internal_lifecycle() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.lifecycle = Lifecycle::Plan {
            target_completion: "v1".into(),
        };
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.lifecycle = Lifecycle::Theorem {
            since: "v1".into(),
        };
        let v = yoneda_equivalent(&s_base, &s_alt, &[adversary()]);
        // Adversary projects only attack_surface + outbound — no
        // lifecycle field exposed.
        assert!(v.equivalent);
    }

    #[test]
    fn stakeholder_sees_persistence_capability_change() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.exposes = vec![Capability::Persist {
            medium: crate::arch::PersistenceMedium::Disk {
                path: "/tmp".into(),
            },
        }];
        let v = yoneda_equivalent(&s_base, &s_alt, &[stakeholder()]);
        assert!(!v.equivalent);
    }

    #[test]
    fn peer_cog_observation_sensitive_to_composes_with() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.composes_with = vec!["core::base".into()];
        let v = yoneda_equivalent(&s_base, &s_alt, &[peer_cog("core::base")]);
        assert!(!v.equivalent);
        // Different peer not affected.
        let v2 = yoneda_equivalent(&s_base, &s_alt, &[peer_cog("core::other")]);
        assert!(v2.equivalent); // composes_with does not contain "core::other" in either
    }

    #[test]
    fn full_canonical_roster_aggregates_disagreements() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
        s_alt.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let v = yoneda_equivalent(&s_base, &s_alt, &[]);
        assert!(!v.equivalent);
        // EndUser (sees exposes), PeerCog (sees exposes/requires),
        // Stakeholder (sees foundation), Auditor (sees everything),
        // Adversary (sees attack_surface=exposes) — all 5 should
        // disagree on at least one of the changed fields.
        assert_eq!(v.disagreement_count, 5);
    }

    #[test]
    fn distinguishing_observers_returns_only_disagreements() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
        let dist = distinguishing_observers(&s_base, &s_alt, &[]);
        // Stakeholder + Auditor see foundation; EndUser/PeerCog/
        // Adversary do not.
        assert_eq!(dist.len(), 2);
        for o in &dist {
            assert!(matches!(
                o,
                Observer::Stakeholder { .. } | Observer::Auditor { .. }
            ));
        }
    }

    #[test]
    fn refactoring_is_trivially_safe_under_yoneda_equivalence() {
        let s = Shape::default_for_unannotated();
        // Pure identity refactoring is trivially safe per §20.7.
        assert!(refactoring_is_trivially_safe(&s, &s, &[]));
    }

    #[test]
    fn refactoring_not_trivially_safe_when_auditor_sees_change() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.strict = false;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.strict = true;
        // Auditor sees `strict` flag — not trivially safe.
        assert!(!refactoring_is_trivially_safe(&s_base, &s_alt, &[auditor()]));
    }

    #[test]
    fn observation_observer_tags_are_stable() {
        // Pin: agent surfaces consume these tags directly per §32.4.
        let s = Shape::default_for_unannotated();
        for obs in [end_user(), peer_cog("any"), stakeholder(), auditor(), adversary()] {
            let projection = observe(&s, &obs);
            assert_eq!(projection.observer_tag(), obs.tag());
        }
    }

    #[test]
    fn agreement_status_tags_stable() {
        assert_eq!(AgreementStatus::Agree.tag(), "agree");
        assert_eq!(AgreementStatus::Disagree.tag(), "disagree");
    }

    #[test]
    fn json_round_trip_preserves_verdict() {
        // Pin: stable schema_version=1 across serde.
        let s_base = Shape::default_for_unannotated();
        let s_alt = Shape::default_for_unannotated();
        let v = yoneda_equivalent(&s_base, &s_alt, &[]);
        let json = serde_json::to_string(&v).expect("must serialise");
        let back: YonedaVerdict =
            serde_json::from_str(&json).expect("must round-trip");
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.equivalent, v.equivalent);
        assert_eq!(back.agreements.len(), v.agreements.len());
    }

    #[test]
    fn architectural_pin_yoneda_lemma_self_observation() {
        // Pin (spec §20.7 + §23): a shape's observation under each
        // canonical observer is structurally identical to itself.
        // This is the algorithmic statement of the Yoneda lemma —
        // `Hom(-, X)(X) ≅ id_X` projected onto observer-functor
        // form.
        let s = Shape::default_for_unannotated();
        for o in Observer::full_canonical_roster() {
            let obs1 = observe(&s, &o);
            let obs2 = observe(&s, &o);
            assert_eq!(obs1, obs2, "observation under {} must be deterministic", o.tag());
        }
    }
}
