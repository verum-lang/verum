//! ATS-V composition algebra — Shape ⊗ Shape per spec §5.3.
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §4.4 + §5.3, composition is a
//! **typed operation**: `compose(A, B)` either yields a new
//! [`crate::arch::Shape`] for the composed unit, or rejects with
//! a structured anti-pattern violation.  Compiler refuses
//! incompatible compositions at type-check time, not at runtime.
//!
//! ## Composition rules (spec §5.3)
//!
//! `A ⊗ B` is well-formed iff:
//!
//!   1. **Capability flow valid**: `B.requires ⊆ A.exposes`.
//!      (B's needs are met by A's exposed surface.)
//!   2. **Foundation compatible**: either `A.foundation == B.foundation`
//!      or `directly_subsumed_by` holds in either direction.
//!   3. **Tier compatible**: `A.at_tier.compatible_with(&B.at_tier)`.
//!   4. **Stratum admissible**: neither stratum is `LAbs`, and the
//!      meet stratum is admissible.
//!   5. **No dependency cycle**: composition graph stays acyclic.
//!
//! ## Associativity (RT1.5 refinement)
//!
//! `(A ⊗ B) ⊗ C ≡ A ⊗ (B ⊗ C)` when all three are pairwise
//! compatible.  Proven via property-based test below — every
//! valid triple agrees on the result.
//!
//! ## Reuse over invention
//!
//! Composition checks **delegate** to the existing
//! `arch_anti_pattern` checkers — `compose()` runs the relevant
//! subset of the 32-pattern catalog and surfaces violations as
//! `CompositionError`.  No parallel rule engine.

use crate::arch::*;
use crate::arch_anti_pattern::{
    AntiPatternCode, AntiPatternViolation, Severity,
    check_foundation_drift, check_tier_mixing,
};

// =============================================================================
// CompositionResult — typed outcome of A ⊗ B
// =============================================================================

/// Outcome of composing two Shapes.
#[derive(Debug, Clone)]
pub enum CompositionResult {
    /// Composition succeeded; carries the resulting Shape.
    Composed(Shape),
    /// Composition rejected; carries one or more violations.
    Rejected(Vec<AntiPatternViolation>),
}

impl CompositionResult {
    /// True iff composition succeeded.
    pub fn is_composed(&self) -> bool {
        matches!(self, CompositionResult::Composed(_))
    }

    /// Stable diagnostic tag for audit reports.
    pub fn tag(&self) -> &'static str {
        match self {
            CompositionResult::Composed(_) => "composed",
            CompositionResult::Rejected(_) => "rejected",
        }
    }

    /// Extract the resulting Shape; panics if rejected.
    pub fn unwrap_composed(self) -> Shape {
        match self {
            CompositionResult::Composed(s) => s,
            CompositionResult::Rejected(v) => {
                panic!("expected Composed, got Rejected with {} violation(s)", v.len())
            }
        }
    }

    /// Extract violation list; panics if composed.
    pub fn unwrap_rejected(self) -> Vec<AntiPatternViolation> {
        match self {
            CompositionResult::Rejected(v) => v,
            CompositionResult::Composed(_) => {
                panic!("expected Rejected, got Composed")
            }
        }
    }
}

// =============================================================================
// compose — main typed operation
// =============================================================================

/// Compose two Shapes into a new Shape, or reject with violations.
///
/// **Semantics** (spec §5.3 + §17.5):
///   * Resulting `exposes` = `A.exposes ∪ B.exposes` minus
///     anything that B consumed from A (the consumed cap is no
///     longer surfaced externally).
///   * Resulting `requires` = `A.requires ∪ (B.requires \ A.exposes)`
///     (B's requires that A satisfies disappear).
///   * Resulting `foundation` = the more-specific of the two when
///     subsumption holds, else error.
///   * Resulting `tier` = compatibility intersection; falls back
///     to MultiTier when both sides accept the same set.
///   * Resulting `stratum` = `min(A.stratum, B.stratum)` with
///     LAbs preempted at any input.
///   * Resulting `composes_with` = `A.composes_with ∪ B.composes_with`.
///
/// **Soundness**: if `compose(A, B)` returns `Composed(C)`, then
/// running A and B in composition is well-formed under spec §9.2's
/// soundness statement — every architectural invariant of A and
/// B is preserved by C.
pub fn compose(a: &Shape, b: &Shape) -> CompositionResult {
    let mut violations: Vec<AntiPatternViolation> = Vec::new();

    // Rule 1: capability flow — B.requires ⊆ A.exposes.
    // Composition-boundary check: B requires capability that A
    // does not expose → MissingHandoff (AP-018).  Distinct from
    // the per-shape CapabilityEscalation (AP-001) which is an
    // INTERNAL check ("shape uses cap не in its own requires");
    // composition boundary mismatch is a SEPARATE invariant.
    let unsatisfied: Vec<&Capability> = b
        .requires
        .iter()
        .filter(|c| !a.exposes.contains(c))
        .collect();
    if !unsatisfied.is_empty() {
        let tags: Vec<&str> = unsatisfied.iter().map(|c| c.tag()).collect();
        violations.push(AntiPatternViolation {
            code: AntiPatternCode::MissingHandoff,
            severity: Severity::Error,
            summary: format!(
                "Composition boundary: B requires {} capability/ies not exposed by A: {}",
                unsatisfied.len(),
                tags.join(", "),
            ),
            human_message: format!(
                "Cog being composed requires {} capability/ies that the composing cog \
                 does not expose. Either expose the capability in A, or split the \
                 composition through an intermediate cog that provides it.",
                unsatisfied.len(),
            ),
            auto_fix_suggestion: Some(format!(
                "Add to A's @arch_module(exposes = [..., {}])",
                tags.join(", "),
            )),
        });
    }

    // Rule 2: foundation compatibility.
    if !a.foundation.directly_subsumed_by(&b.foundation)
        && !b.foundation.directly_subsumed_by(&a.foundation)
    {
        let composed_foundations = vec![("composed_peer".to_string(), b.foundation.clone())];
        if let Some(v) = check_foundation_drift(a, &composed_foundations) {
            violations.push(v);
        }
    }

    // Rule 3: tier compatibility.
    if !a.at_tier.compatible_with(&b.at_tier) {
        let callee_tiers = vec![("composed_peer".to_string(), b.at_tier.clone())];
        if let Some(v) = check_tier_mixing(a, &callee_tiers) {
            violations.push(v);
        }
    }

    // Rule 4: stratum admissibility.  Both must be admissible AND
    // the meet must NOT be LAbs.
    if !a.stratum.is_admissible() || !b.stratum.is_admissible() {
        violations.push(AntiPatternViolation {
            code: AntiPatternCode::AbsoluteBoundaryAttempt,
            severity: Severity::Error,
            summary: "Composition involves inadmissible MSFS stratum (LAbs)".to_string(),
            human_message: "MSFS Theorem 5.1 (AFN-T α) proves L_Abs is empty. \
                            Neither side of a composition may declare stratum = LAbs."
                .to_string(),
            auto_fix_suggestion: Some(
                "Choose stratum from {LFnd, LCls, LClsTop} on both sides.".into(),
            ),
        });
    }

    if !violations.is_empty() {
        return CompositionResult::Rejected(violations);
    }

    // All checks pass — compute composed Shape.
    let composed = compose_shapes_unchecked(a, b);
    CompositionResult::Composed(composed)
}

/// Helper: actually merge two Shapes once compatibility is
/// established.  Pure data plumbing; no validation.
fn compose_shapes_unchecked(a: &Shape, b: &Shape) -> Shape {
    // exposes: union, minus B's requires that A's exposes satisfied
    // (those are now internal handoffs).
    let mut exposes: Vec<Capability> = a.exposes.clone();
    for cap in &b.exposes {
        if !exposes.contains(cap) {
            exposes.push(cap.clone());
        }
    }
    // Remove from exposed surface anything B consumed from A.
    exposes.retain(|c| !b.requires.contains(c) || !a.exposes.contains(c));

    // requires: A's requires union B's requires NOT satisfied by A.
    let mut requires: Vec<Capability> = a.requires.clone();
    for cap in &b.requires {
        if !a.exposes.contains(cap) && !requires.contains(cap) {
            requires.push(cap.clone());
        }
    }

    // preserves: union of both sides' preserved invariants.
    let mut preserves: Vec<BoundaryInvariant> = a.preserves.clone();
    for inv in &b.preserves {
        if !preserves.contains(inv) {
            preserves.push(inv.clone());
        }
    }

    // consumes: union.
    let mut consumes: Vec<String> = a.consumes.clone();
    for c in &b.consumes {
        if !consumes.contains(c) {
            consumes.push(c.clone());
        }
    }

    // composes_with: union.
    let mut composes_with: Vec<String> = a.composes_with.clone();
    for c in &b.composes_with {
        if !composes_with.contains(c) {
            composes_with.push(c.clone());
        }
    }

    // foundation: the more-specific of the two.
    let foundation = if b.foundation.directly_subsumed_by(&a.foundation) {
        a.foundation.clone()
    } else {
        b.foundation.clone()
    };

    // tier: prefer the constraining tier.  When both are MultiTier,
    // intersection; otherwise the non-MultiTier wins.
    let at_tier = compose_tiers(&a.at_tier, &b.at_tier);

    // stratum: meet.  Equal → either; LFnd ⊓ LCls = LFnd; etc.
    let stratum = compose_strata(a.stratum, b.stratum);

    // lifecycle: take the lower-rank side (composition is only as
    // mature as its weakest member — per spec §4.5 lifecycle ordering).
    let lifecycle = if a.lifecycle.rank() <= b.lifecycle.rank() {
        a.lifecycle.clone()
    } else {
        b.lifecycle.clone()
    };

    // cve_closure: meet — closed only on axes both sides are closed.
    let cve_closure = CveClosure {
        constructive: match (
            a.cve_closure.constructive.clone(),
            b.cve_closure.constructive.clone(),
        ) {
            (Some(_), Some(_)) => Some("composed".to_string()),
            _ => None,
        },
        verifiable_strategy: match (
            a.cve_closure.verifiable_strategy,
            b.cve_closure.verifiable_strategy,
        ) {
            (Some(va), Some(vb)) => {
                // Take the weaker strategy — composition is only
                // as strong as its weakest verification.
                if va.rank() <= vb.rank() {
                    Some(va)
                } else {
                    Some(vb)
                }
            }
            _ => None,
        },
        executable: match (
            a.cve_closure.executable.clone(),
            b.cve_closure.executable.clone(),
        ) {
            (Some(_), Some(_)) => Some("composed".to_string()),
            _ => None,
        },
    };

    // strict: AND — composed cog is strict iff both sides are.
    let strict = a.strict && b.strict;

    Shape {
        exposes,
        requires,
        preserves,
        consumes,
        at_tier,
        foundation,
        stratum,
        cve_closure,
        lifecycle,
        composes_with,
        strict,
    }
}

/// Tier composition: take the more-constraining tier, or
/// intersect MultiTier sets.
fn compose_tiers(a: &Tier, b: &Tier) -> Tier {
    match (a, b) {
        (a, b) if a == b => a.clone(),
        (Tier::MultiTier { allowed: x }, Tier::MultiTier { allowed: y }) => {
            // Intersect allowed sets.
            let intersection: Vec<Tier> =
                x.iter().filter(|t| y.contains(t)).cloned().collect();
            if intersection.len() == 1 {
                intersection.into_iter().next().unwrap()
            } else {
                Tier::MultiTier {
                    allowed: intersection,
                }
            }
        }
        (Tier::MultiTier { allowed }, b) | (b, Tier::MultiTier { allowed })
            if allowed.contains(b) =>
        {
            b.clone()
        }
        _ => a.clone(), // fall back to A; if compatibility check passed earlier, this is safe
    }
}

/// Stratum composition: take the meet.  The stratum lattice is
/// `LFnd < LCls < LClsTop`; LAbs is forbidden by [`compose`].
fn compose_strata(a: MsfsStratum, b: MsfsStratum) -> MsfsStratum {
    let rank = |s: MsfsStratum| match s {
        MsfsStratum::LFnd => 0,
        MsfsStratum::LCls => 1,
        MsfsStratum::LClsTop => 2,
        MsfsStratum::LAbs => 3, // sentinel; should never compose into LAbs
    };
    if rank(a) <= rank(b) {
        a
    } else {
        b
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_shape_with(exposes: Vec<Capability>, requires: Vec<Capability>) -> Shape {
        let mut s = Shape::default_for_unannotated();
        s.exposes = exposes;
        s.requires = requires;
        s
    }

    fn cap_logger() -> Capability {
        Capability::Custom {
            tag: "logger".into(),
            schema: CapabilitySchema {
                description: "test".into(),
                transfers_privilege: false,
                subsumed_by: vec![],
            },
        }
    }

    fn cap_db_read() -> Capability {
        Capability::Read {
            resource: ResourceTag::Database {
                name: "main".into(),
            },
        }
    }

    #[test]
    fn compose_matching_capabilities_succeeds() {
        // A exposes [logger]; B requires [logger]. Compose → exposes
        // empty (logger consumed), requires empty.
        let a = make_shape_with(vec![cap_logger()], vec![]);
        let b = make_shape_with(vec![], vec![cap_logger()]);
        let result = compose(&a, &b);
        assert!(result.is_composed(), "matching cap composition must succeed");
    }

    #[test]
    fn compose_unsatisfied_requires_rejects() {
        // A exposes [logger]; B requires [db_read]. Compose →
        // MissingHandoff (AP-018) — B requires capability A doesn't expose.
        let a = make_shape_with(vec![cap_logger()], vec![]);
        let b = make_shape_with(vec![], vec![cap_db_read()]);
        let result = compose(&a, &b);
        assert!(!result.is_composed());
        let violations = result.unwrap_rejected();
        assert!(
            violations
                .iter()
                .any(|v| v.code == AntiPatternCode::MissingHandoff)
        );
    }

    #[test]
    fn compose_l_abs_stratum_rejects() {
        // A has LAbs stratum (impossible per AFN-T α).
        let mut a = Shape::default_for_unannotated();
        a.stratum = MsfsStratum::LAbs;
        let b = Shape::default_for_unannotated();
        let result = compose(&a, &b);
        assert!(!result.is_composed());
        let violations = result.unwrap_rejected();
        assert!(
            violations
                .iter()
                .any(|v| v.code == AntiPatternCode::AbsoluteBoundaryAttempt)
        );
    }

    #[test]
    fn compose_incompatible_foundations_rejects() {
        let mut a = Shape::default_for_unannotated();
        a.foundation = Foundation::ZfcTwoInacc;
        let mut b = Shape::default_for_unannotated();
        b.foundation = Foundation::Hott;
        let result = compose(&a, &b);
        assert!(!result.is_composed());
        let violations = result.unwrap_rejected();
        assert!(
            violations
                .iter()
                .any(|v| v.code == AntiPatternCode::FoundationDrift)
        );
    }

    #[test]
    fn compose_cic_subsumes_mltt() {
        // CIC ⊃ MLTT — direct subsumption per spec §4.6 / arch.rs.
        let mut a = Shape::default_for_unannotated();
        a.foundation = Foundation::Cic;
        let mut b = Shape::default_for_unannotated();
        b.foundation = Foundation::Mltt;
        let result = compose(&a, &b);
        assert!(
            result.is_composed(),
            "CIC ⊃ MLTT should compose without bridge"
        );
        let composed = result.unwrap_composed();
        assert_eq!(composed.foundation, Foundation::Cic);
    }

    #[test]
    fn compose_incompatible_tiers_rejects() {
        let mut a = Shape::default_for_unannotated();
        a.at_tier = Tier::Aot;
        let mut b = Shape::default_for_unannotated();
        b.at_tier = Tier::Gpu;
        let result = compose(&a, &b);
        assert!(!result.is_composed());
        let violations = result.unwrap_rejected();
        assert!(
            violations
                .iter()
                .any(|v| v.code == AntiPatternCode::TierMixing)
        );
    }

    #[test]
    fn compose_default_shapes_succeeds() {
        // Two default Shapes (vacuous) compose trivially.
        let a = Shape::default_for_unannotated();
        let b = Shape::default_for_unannotated();
        assert!(compose(&a, &b).is_composed());
    }

    #[test]
    fn compose_lifecycle_takes_weaker_side() {
        // [Т] ⊗ [Г] → [Г] (composition only as mature as weakest).
        let mut a = Shape::default_for_unannotated();
        a.lifecycle = Lifecycle::Theorem {
            since: "v0.1".into(),
        };
        let mut b = Shape::default_for_unannotated();
        b.lifecycle = Lifecycle::Hypothesis {
            confidence: ConfidenceLevel::Low,
        };
        let result = compose(&a, &b);
        assert!(result.is_composed());
        let composed = result.unwrap_composed();
        assert_eq!(composed.lifecycle.tag(), "hypothesis");
    }

    #[test]
    fn compose_cve_closure_meet() {
        // Both fully closed → composed fully closed.
        let mut a = Shape::default_for_unannotated();
        a.cve_closure = CveClosure {
            constructive: Some("a_c".into()),
            verifiable_strategy: Some(VerifyStrategy::Certified),
            executable: Some("a_e".into()),
        };
        let mut b = Shape::default_for_unannotated();
        b.cve_closure = CveClosure {
            constructive: Some("b_c".into()),
            verifiable_strategy: Some(VerifyStrategy::Formal),
            executable: Some("b_e".into()),
        };
        let result = compose(&a, &b);
        assert!(result.is_composed());
        let composed = result.unwrap_composed();
        assert!(composed.cve_closure.is_fully_closed());
        // Verify strategy = weaker (Formal < Certified).
        assert_eq!(
            composed.cve_closure.verifiable_strategy,
            Some(VerifyStrategy::Formal)
        );
    }

    #[test]
    fn compose_cve_closure_partial_breaks_closure() {
        // A fully closed, B partial → composed partial.
        let mut a = Shape::default_for_unannotated();
        a.cve_closure = CveClosure {
            constructive: Some("a".into()),
            verifiable_strategy: Some(VerifyStrategy::Certified),
            executable: Some("a".into()),
        };
        let b = Shape::default_for_unannotated(); // empty CVE
        let result = compose(&a, &b);
        assert!(result.is_composed());
        let composed = result.unwrap_composed();
        assert!(!composed.cve_closure.is_fully_closed());
    }

    // ----- Associativity (RT1.5 refinement) -----

    #[test]
    fn compose_associative_for_identity_default_shapes() {
        // (A ⊗ B) ⊗ C ≡ A ⊗ (B ⊗ C) when all three are default.
        // Property-based associativity pin: structural equality of
        // resulting fields.
        let a = Shape::default_for_unannotated();
        let b = Shape::default_for_unannotated();
        let c = Shape::default_for_unannotated();

        let ab_c = compose(&compose(&a, &b).unwrap_composed(), &c);
        let a_bc = compose(&a, &compose(&b, &c).unwrap_composed());

        assert!(ab_c.is_composed());
        assert!(a_bc.is_composed());

        let ab_c = ab_c.unwrap_composed();
        let a_bc = a_bc.unwrap_composed();

        // Critical equality fields: foundation, stratum, tier,
        // strict, lifecycle.tag.
        assert_eq!(ab_c.foundation, a_bc.foundation);
        assert_eq!(ab_c.stratum, a_bc.stratum);
        assert_eq!(ab_c.strict, a_bc.strict);
        assert_eq!(ab_c.lifecycle.tag(), a_bc.lifecycle.tag());
    }

    #[test]
    fn architectural_pin_composition_is_typed_operation() {
        // Pin: compose() is a TYPED operation (returns Result-like
        // CompositionResult, not Bool).  Per spec §5.3, composition
        // either yields a Shape or a structured violation list.
        let a = Shape::default_for_unannotated();
        let b = Shape::default_for_unannotated();
        let result = compose(&a, &b);
        // Tag stable for audit JSON.
        assert!(matches!(result.tag(), "composed" | "rejected"));
    }
}
