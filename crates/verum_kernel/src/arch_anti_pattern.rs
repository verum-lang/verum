//! ATS-V anti-pattern catalog — kernel-side refinement predicates.
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §7 (Anti-pattern catalog) +
//! §32.4 (Stable error codes), each canonical anti-pattern has:
//!
//!   * Stable RFC error code `ATS-V-AP-NNN` (machine-readable).
//!   * Refinement predicate over [`crate::arch::Shape`] (algorithmic
//!     check).
//!   * Structured diagnostic JSON shape (dual-audience: human
//!     `human_message` + agent `auto_fix_diff`).
//!   * Docs URL (`https://verum.lang/docs/ats-v/ap-NNN`).
//!
//! This module ships the first 10 canonical anti-patterns
//! (Сезон 1.2).  Remaining 22 land in Сезон 2 (per spec §11).
//!
//! ## Discharge route
//!
//! Each anti-pattern is checked by a `check_*` function returning
//! `Option<AntiPatternViolation>`.  `None` means the predicate
//! holds (no violation); `Some(v)` carries the structured diagnostic.
//!
//! [`check_all_anti_patterns`] walks every check_* function over
//! a given Shape and returns all violations — used by the
//! ATS-V phase + audit gate.
//!
//! ## Stable error code reservation
//!
//! Codes ATS-V-AP-001..010 are RESERVED for the patterns below;
//! adding new patterns appends to the catalog (ATS-V-AP-011+).
//! Removing a pattern requires deprecation cycle ≥ 2 minor
//! versions — codes never get re-used (per spec §29.5 versioning).

use crate::arch::{Capability, Foundation, Lifecycle, MsfsStratum, Shape, Tier};

// =============================================================================
// AntiPatternCode — stable RFC code
// =============================================================================

/// Stable RFC error code `ATS-V-AP-NNN`.  Pattern-matchable by
/// agents; documented in spec + on docs site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AntiPatternCode {
    // ----- Сезон 1 (AP-001..010) — capability/composition core -----
    CapabilityEscalation,    // ATS-V-AP-001
    CapabilityLeak,          // ATS-V-AP-002
    DependencyCycle,         // ATS-V-AP-003
    TierMixing,              // ATS-V-AP-004
    FoundationDrift,         // ATS-V-AP-005
    RegisterMixing,          // ATS-V-AP-006
    TxStraddling,            // ATS-V-AP-007
    ResourceStraddling,      // ATS-V-AP-008
    LifecycleRegression,     // ATS-V-AP-009
    CveIncomplete,           // ATS-V-AP-010

    // ----- Сезон 2 base (AP-011..026) — boundary / lifecycle / capability ontology -----
    AbsoluteBoundaryAttempt,           // ATS-V-AP-011 — stratum = LAbs (AFN-T α violation)
    InvariantViolation,                // ATS-V-AP-012 — boundary invariant not preserved
    DanglingMessageType,               // ATS-V-AP-013 — message type без wire encoding
    UnauthenticatedCrossing,           // ATS-V-AP-014 — Network boundary без AuthenticatedFirst
    DeterministicViolation,            // ATS-V-AP-015 — DST test использует non-deterministic
    CapabilityDuplication,             // ATS-V-AP-016 — Linear cap duplicated
    OrphanCapability,                  // ATS-V-AP-017 — Relevant cap отброшена
    MissingHandoff,                    // ATS-V-AP-018 — composition cap не в composes_with
    FoundationDowngrade,               // ATS-V-AP-019 — strong foundation → weak без bridge
    TimeBoundLeakage,                  // ATS-V-AP-020 — TimeBound cap живёт > TTL
    PersistenceMismatch,               // ATS-V-AP-021 — Persist cap для non-durable op
    CapabilityLaundering,              // ATS-V-AP-022 — multi-hop privilege escalation
    FoundationForgery,                 // ATS-V-AP-023 — declared foundation ≠ cited
    TransitiveLifecycleRegression,     // ATS-V-AP-024 — transitive [Т]→...→[Г]
    DeclarationDrift,                  // ATS-V-AP-025 — declared shape ≠ inferred
    FoundationContentMismatch,         // ATS-V-AP-026 — code-content uses foreign foundation

    // ----- Сезон 2 MTAC (AP-027..032) — modal-temporal-architectural calculus -----
    TemporalInconsistency,             // ATS-V-AP-027 — invariant breaks across time
    CounterfactualBrittleness,         // ATS-V-AP-028 — fragile under decision swap
    MissedAdjoint,                     // ATS-V-AP-029 — refactoring без inverse
    UniversalPropertyViolation,        // ATS-V-AP-030 — claimed unique но не уникален
    PhantomEvolution,                  // ATS-V-AP-031 — evolution path с unsat trigger
    YonedaInequivalentRefactor,        // ATS-V-AP-032 — refactor changes observer-functor
}

impl AntiPatternCode {
    /// Stable error code string `ATS-V-AP-NNN`.
    pub fn code(&self) -> &'static str {
        match self {
            AntiPatternCode::CapabilityEscalation => "ATS-V-AP-001",
            AntiPatternCode::CapabilityLeak => "ATS-V-AP-002",
            AntiPatternCode::DependencyCycle => "ATS-V-AP-003",
            AntiPatternCode::TierMixing => "ATS-V-AP-004",
            AntiPatternCode::FoundationDrift => "ATS-V-AP-005",
            AntiPatternCode::RegisterMixing => "ATS-V-AP-006",
            AntiPatternCode::TxStraddling => "ATS-V-AP-007",
            AntiPatternCode::ResourceStraddling => "ATS-V-AP-008",
            AntiPatternCode::LifecycleRegression => "ATS-V-AP-009",
            AntiPatternCode::CveIncomplete => "ATS-V-AP-010",
            AntiPatternCode::AbsoluteBoundaryAttempt => "ATS-V-AP-011",
            AntiPatternCode::InvariantViolation => "ATS-V-AP-012",
            AntiPatternCode::DanglingMessageType => "ATS-V-AP-013",
            AntiPatternCode::UnauthenticatedCrossing => "ATS-V-AP-014",
            AntiPatternCode::DeterministicViolation => "ATS-V-AP-015",
            AntiPatternCode::CapabilityDuplication => "ATS-V-AP-016",
            AntiPatternCode::OrphanCapability => "ATS-V-AP-017",
            AntiPatternCode::MissingHandoff => "ATS-V-AP-018",
            AntiPatternCode::FoundationDowngrade => "ATS-V-AP-019",
            AntiPatternCode::TimeBoundLeakage => "ATS-V-AP-020",
            AntiPatternCode::PersistenceMismatch => "ATS-V-AP-021",
            AntiPatternCode::CapabilityLaundering => "ATS-V-AP-022",
            AntiPatternCode::FoundationForgery => "ATS-V-AP-023",
            AntiPatternCode::TransitiveLifecycleRegression => "ATS-V-AP-024",
            AntiPatternCode::DeclarationDrift => "ATS-V-AP-025",
            AntiPatternCode::FoundationContentMismatch => "ATS-V-AP-026",
            AntiPatternCode::TemporalInconsistency => "ATS-V-AP-027",
            AntiPatternCode::CounterfactualBrittleness => "ATS-V-AP-028",
            AntiPatternCode::MissedAdjoint => "ATS-V-AP-029",
            AntiPatternCode::UniversalPropertyViolation => "ATS-V-AP-030",
            AntiPatternCode::PhantomEvolution => "ATS-V-AP-031",
            AntiPatternCode::YonedaInequivalentRefactor => "ATS-V-AP-032",
        }
    }

    /// Canonical short name (matches spec §7 + §26 catalog).
    pub fn name(&self) -> &'static str {
        match self {
            AntiPatternCode::CapabilityEscalation => "CapabilityEscalation",
            AntiPatternCode::CapabilityLeak => "CapabilityLeak",
            AntiPatternCode::DependencyCycle => "DependencyCycle",
            AntiPatternCode::TierMixing => "TierMixing",
            AntiPatternCode::FoundationDrift => "FoundationDrift",
            AntiPatternCode::RegisterMixing => "RegisterMixing",
            AntiPatternCode::TxStraddling => "TxStraddling",
            AntiPatternCode::ResourceStraddling => "ResourceStraddling",
            AntiPatternCode::LifecycleRegression => "LifecycleRegression",
            AntiPatternCode::CveIncomplete => "CveIncomplete",
            AntiPatternCode::AbsoluteBoundaryAttempt => "AbsoluteBoundaryAttempt",
            AntiPatternCode::InvariantViolation => "InvariantViolation",
            AntiPatternCode::DanglingMessageType => "DanglingMessageType",
            AntiPatternCode::UnauthenticatedCrossing => "UnauthenticatedCrossing",
            AntiPatternCode::DeterministicViolation => "DeterministicViolation",
            AntiPatternCode::CapabilityDuplication => "CapabilityDuplication",
            AntiPatternCode::OrphanCapability => "OrphanCapability",
            AntiPatternCode::MissingHandoff => "MissingHandoff",
            AntiPatternCode::FoundationDowngrade => "FoundationDowngrade",
            AntiPatternCode::TimeBoundLeakage => "TimeBoundLeakage",
            AntiPatternCode::PersistenceMismatch => "PersistenceMismatch",
            AntiPatternCode::CapabilityLaundering => "CapabilityLaundering",
            AntiPatternCode::FoundationForgery => "FoundationForgery",
            AntiPatternCode::TransitiveLifecycleRegression => "TransitiveLifecycleRegression",
            AntiPatternCode::DeclarationDrift => "DeclarationDrift",
            AntiPatternCode::FoundationContentMismatch => "FoundationContentMismatch",
            AntiPatternCode::TemporalInconsistency => "TemporalInconsistency",
            AntiPatternCode::CounterfactualBrittleness => "CounterfactualBrittleness",
            AntiPatternCode::MissedAdjoint => "MissedAdjoint",
            AntiPatternCode::UniversalPropertyViolation => "UniversalPropertyViolation",
            AntiPatternCode::PhantomEvolution => "PhantomEvolution",
            AntiPatternCode::YonedaInequivalentRefactor => "YonedaInequivalentRefactor",
        }
    }

    /// Documentation URL — stable per spec §32.4.  Format
    /// `https://verum.lang/docs/ats-v/ap-NNN`.
    pub fn docs_url(&self) -> String {
        let n = match self {
            AntiPatternCode::CapabilityEscalation => 1,
            AntiPatternCode::CapabilityLeak => 2,
            AntiPatternCode::DependencyCycle => 3,
            AntiPatternCode::TierMixing => 4,
            AntiPatternCode::FoundationDrift => 5,
            AntiPatternCode::RegisterMixing => 6,
            AntiPatternCode::TxStraddling => 7,
            AntiPatternCode::ResourceStraddling => 8,
            AntiPatternCode::LifecycleRegression => 9,
            AntiPatternCode::CveIncomplete => 10,
            AntiPatternCode::AbsoluteBoundaryAttempt => 11,
            AntiPatternCode::InvariantViolation => 12,
            AntiPatternCode::DanglingMessageType => 13,
            AntiPatternCode::UnauthenticatedCrossing => 14,
            AntiPatternCode::DeterministicViolation => 15,
            AntiPatternCode::CapabilityDuplication => 16,
            AntiPatternCode::OrphanCapability => 17,
            AntiPatternCode::MissingHandoff => 18,
            AntiPatternCode::FoundationDowngrade => 19,
            AntiPatternCode::TimeBoundLeakage => 20,
            AntiPatternCode::PersistenceMismatch => 21,
            AntiPatternCode::CapabilityLaundering => 22,
            AntiPatternCode::FoundationForgery => 23,
            AntiPatternCode::TransitiveLifecycleRegression => 24,
            AntiPatternCode::DeclarationDrift => 25,
            AntiPatternCode::FoundationContentMismatch => 26,
            AntiPatternCode::TemporalInconsistency => 27,
            AntiPatternCode::CounterfactualBrittleness => 28,
            AntiPatternCode::MissedAdjoint => 29,
            AntiPatternCode::UniversalPropertyViolation => 30,
            AntiPatternCode::PhantomEvolution => 31,
            AntiPatternCode::YonedaInequivalentRefactor => 32,
        };
        format!("https://verum.lang/docs/ats-v/ap-{:03}", n)
    }

    /// Which Сезон / season introduced this pattern.  Stable for
    /// version-compat tracking (per spec §29.5 versioning policy).
    pub fn season(&self) -> u8 {
        match self {
            // Сезон 1 — AP-001..010
            AntiPatternCode::CapabilityEscalation
            | AntiPatternCode::CapabilityLeak
            | AntiPatternCode::DependencyCycle
            | AntiPatternCode::TierMixing
            | AntiPatternCode::FoundationDrift
            | AntiPatternCode::RegisterMixing
            | AntiPatternCode::TxStraddling
            | AntiPatternCode::ResourceStraddling
            | AntiPatternCode::LifecycleRegression
            | AntiPatternCode::CveIncomplete => 1,
            // Сезон 2 — AP-011..032 (base + MTAC)
            _ => 2,
        }
    }

    /// True iff the pattern is MTAC-specific (modal-temporal,
    /// per spec §20-§23 + §26).
    pub fn is_mtac(&self) -> bool {
        matches!(
            self,
            AntiPatternCode::TemporalInconsistency
                | AntiPatternCode::CounterfactualBrittleness
                | AntiPatternCode::MissedAdjoint
                | AntiPatternCode::UniversalPropertyViolation
                | AntiPatternCode::PhantomEvolution
                | AntiPatternCode::YonedaInequivalentRefactor
        )
    }

    /// Full canonical list — Сезон 1+2 = 32 patterns total.
    pub fn full_list() -> [AntiPatternCode; 32] {
        [
            // Сезон 1 (10)
            AntiPatternCode::CapabilityEscalation,
            AntiPatternCode::CapabilityLeak,
            AntiPatternCode::DependencyCycle,
            AntiPatternCode::TierMixing,
            AntiPatternCode::FoundationDrift,
            AntiPatternCode::RegisterMixing,
            AntiPatternCode::TxStraddling,
            AntiPatternCode::ResourceStraddling,
            AntiPatternCode::LifecycleRegression,
            AntiPatternCode::CveIncomplete,
            // Сезон 2 base (16)
            AntiPatternCode::AbsoluteBoundaryAttempt,
            AntiPatternCode::InvariantViolation,
            AntiPatternCode::DanglingMessageType,
            AntiPatternCode::UnauthenticatedCrossing,
            AntiPatternCode::DeterministicViolation,
            AntiPatternCode::CapabilityDuplication,
            AntiPatternCode::OrphanCapability,
            AntiPatternCode::MissingHandoff,
            AntiPatternCode::FoundationDowngrade,
            AntiPatternCode::TimeBoundLeakage,
            AntiPatternCode::PersistenceMismatch,
            AntiPatternCode::CapabilityLaundering,
            AntiPatternCode::FoundationForgery,
            AntiPatternCode::TransitiveLifecycleRegression,
            AntiPatternCode::DeclarationDrift,
            AntiPatternCode::FoundationContentMismatch,
            // Сезон 2 MTAC (6)
            AntiPatternCode::TemporalInconsistency,
            AntiPatternCode::CounterfactualBrittleness,
            AntiPatternCode::MissedAdjoint,
            AntiPatternCode::UniversalPropertyViolation,
            AntiPatternCode::PhantomEvolution,
            AntiPatternCode::YonedaInequivalentRefactor,
        ]
    }
}

// =============================================================================
// AntiPatternViolation — structured diagnostic
// =============================================================================

/// Structured diagnostic produced when an anti-pattern check fails.
/// Per spec §32.4 — dual-audience: `human_message` for review,
/// `auto_fix_suggestion` for agent automated remediation.
#[derive(Debug, Clone)]
pub struct AntiPatternViolation {
    /// Stable RFC code.
    pub code: AntiPatternCode,
    /// Severity (anti-patterns default to error in strict mode,
    /// warning in soft).
    pub severity: Severity,
    /// One-line summary (machine-friendly).
    pub summary: String,
    /// Human-friendly message (review-friendly).
    pub human_message: String,
    /// Auto-fix hint (agent-actionable).
    pub auto_fix_suggestion: Option<String>,
}

impl AntiPatternViolation {
    /// Convert to canonical `VerificationVerdict::Rejected` per the
    /// foundation type in `verum_kernel::verdict`.  Used when the
    /// ATS-V phase wants to surface the anti-pattern through the
    /// canonical verdict pipeline.
    pub fn into_verdict(self) -> crate::verdict::VerificationVerdict {
        use crate::verdict::*;
        VerificationVerdict::Rejected {
            method: DischargeMethod::AtsVAntiPatternCheck {
                pattern_tag: self.code.name(),
            },
            counterexample: Counterexample::from_summary(self.summary)
                .with("code", self.code.code())
                .with("docs_url", self.code.docs_url())
                .with("severity", self.severity.tag())
                .with("human_message", self.human_message),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
    Hint,
}

impl Severity {
    pub fn tag(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Hint => "hint",
        }
    }
}

// =============================================================================
// Anti-pattern checks — per-pattern refinement predicates
// =============================================================================

/// ATS-V-AP-001 — CapabilityEscalation.
/// Cog uses a capability not declared in `requires`.
///
/// **Predicate**: `forall c ∈ used_capabilities. c ∈ requires`.
/// (Used capabilities are inferred from the cog source by the
/// ATS-V phase; this checker takes the inferred set as input.)
pub fn check_capability_escalation(
    shape: &Shape,
    inferred_used: &[Capability],
) -> Option<AntiPatternViolation> {
    let undeclared: Vec<&Capability> = inferred_used
        .iter()
        .filter(|c| !shape.requires.contains(c))
        .collect();
    if undeclared.is_empty() {
        return None;
    }
    let undeclared_tags: Vec<&str> = undeclared.iter().map(|c| c.tag()).collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::CapabilityEscalation,
        severity: if shape.strict {
            Severity::Error
        } else {
            Severity::Warning
        },
        summary: format!(
            "Capability/ies not declared in @arch_module(requires): {}",
            undeclared_tags.join(", "),
        ),
        human_message: format!(
            "Cog uses {} capability/ies that are not declared in its @arch_module(requires). \
             Add them to the requires list, or remove the usage.",
            undeclared.len(),
        ),
        auto_fix_suggestion: Some(format!(
            "Add to @arch_module(requires = [..., {}])",
            undeclared_tags.join(", "),
        )),
    })
}

/// ATS-V-AP-002 — CapabilityLeak.
/// Linear/affine capability passed beyond its scope without
/// explicit handoff.
///
/// **Predicate**: `forall c : Linear ∈ uses(cog). c.scope ⊆ cog.scope`.
/// (Scope analysis is delegated to the ATS-V phase; this checker
/// receives a list of capabilities flagged as escaping.)
pub fn check_capability_leak(
    shape: &Shape,
    leaked_capabilities: &[Capability],
) -> Option<AntiPatternViolation> {
    if leaked_capabilities.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::CapabilityLeak,
        severity: Severity::Error,
        summary: format!(
            "{} linear/affine capability/ies escape their scope",
            leaked_capabilities.len(),
        ),
        human_message: "A capability marked linear or affine via @quantity(1) was passed beyond its declared scope. \
                        Linear capabilities must be consumed exactly once within their issuing scope."
            .to_string(),
        auto_fix_suggestion: Some(
            "Either consume the capability within scope, or change @quantity(1) to @quantity(omega) if duplication is acceptable."
                .to_string(),
        ),
    })
    .filter(|_| !shape.consumes.is_empty() || !leaked_capabilities.is_empty())
}

/// ATS-V-AP-003 — DependencyCycle.
/// `composes_with` graph contains a cycle.
///
/// **Predicate**: `acyclic(import_graph)`.
pub fn check_dependency_cycle(
    shape: &Shape,
    cog_name: &str,
    composes_graph: &[(String, Vec<String>)],
) -> Option<AntiPatternViolation> {
    let _ = shape;
    // Tarjan-style SCC: any cycle involving `cog_name` is a
    // violation.
    if has_cycle_involving(cog_name, composes_graph) {
        return Some(AntiPatternViolation {
            code: AntiPatternCode::DependencyCycle,
            severity: Severity::Error,
            summary: format!("Cog {} participates in a dependency cycle", cog_name),
            human_message: format!(
                "Cog {} appears in a cycle of @arch_module(composes_with) declarations. \
                 Architectural composition graphs must be acyclic.",
                cog_name,
            ),
            auto_fix_suggestion: Some(
                "Break the cycle by introducing a protocol boundary or a separate compositional layer."
                    .into(),
            ),
        });
    }
    None
}

/// Helper: cycle detection in the composition graph (DFS-based).
fn has_cycle_involving(cog_name: &str, edges: &[(String, Vec<String>)]) -> bool {
    use std::collections::{HashMap, HashSet};
    let graph: HashMap<&str, &[String]> = edges
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_slice()))
        .collect();
    let mut visiting: HashSet<&str> = HashSet::new();
    let mut visited: HashSet<&str> = HashSet::new();
    fn dfs<'a>(
        node: &'a str,
        graph: &HashMap<&'a str, &'a [String]>,
        visiting: &mut std::collections::HashSet<&'a str>,
        visited: &mut std::collections::HashSet<&'a str>,
    ) -> bool {
        if visiting.contains(node) {
            return true; // cycle found
        }
        if visited.contains(node) {
            return false;
        }
        visiting.insert(node);
        if let Some(neighbours) = graph.get(node) {
            for n in *neighbours {
                if dfs(n.as_str(), graph, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(node);
        visited.insert(node);
        false
    }
    dfs(cog_name, &graph, &mut visiting, &mut visited)
}

/// ATS-V-AP-004 — TierMixing.
/// A function in tier A calls a function in tier B without bridge.
///
/// **Predicate**: `forall fn_call. tier_compat(caller.at_tier, callee.at_tier)`.
pub fn check_tier_mixing(
    shape: &Shape,
    callee_tiers: &[(String, Tier)],
) -> Option<AntiPatternViolation> {
    let incompat: Vec<(&str, &Tier)> = callee_tiers
        .iter()
        .filter(|(_, callee)| !shape.at_tier.compatible_with(callee))
        .map(|(name, t)| (name.as_str(), t))
        .collect();
    if incompat.is_empty() {
        return None;
    }
    let names: Vec<String> = incompat
        .iter()
        .map(|(n, t)| format!("{} ({})", n, t.tag()))
        .collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::TierMixing,
        severity: Severity::Error,
        summary: format!(
            "Tier {} cog calls into incompatible tier(s): {}",
            shape.at_tier.tag(),
            names.join(", "),
        ),
        human_message: format!(
            "This cog runs at @arch_module(at_tier = {}), but it calls into functions at incompatible tiers. \
             Tier mixing requires an explicit @arch_tier_bridge annotation.",
            shape.at_tier.tag(),
        ),
        auto_fix_suggestion: Some(
            "Either change at_tier to MultiTier with the called tiers included, or introduce an arch_tier_bridge."
                .into(),
        ),
    })
}

/// ATS-V-AP-005 — FoundationDrift.
/// Composition with a cog of incompatible foundation, no bridge.
///
/// **Predicate**: `forall (A, B) ∈ composes. A.foundation = B.foundation ∨ ∃ bridge(A, B)`.
pub fn check_foundation_drift(
    shape: &Shape,
    composed_foundations: &[(String, Foundation)],
) -> Option<AntiPatternViolation> {
    let drifted: Vec<(&str, &Foundation)> = composed_foundations
        .iter()
        .filter(|(_, f)| !shape.foundation.directly_subsumed_by(f) && !f.directly_subsumed_by(&shape.foundation))
        .map(|(n, f)| (n.as_str(), f))
        .collect();
    if drifted.is_empty() {
        return None;
    }
    let names: Vec<String> = drifted
        .iter()
        .map(|(n, f)| format!("{} ({})", n, f.tag()))
        .collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::FoundationDrift,
        severity: Severity::Error,
        summary: format!(
            "Foundation {} composed with incompatible foundation(s): {}",
            shape.foundation.tag(),
            names.join(", "),
        ),
        human_message: format!(
            "This cog uses foundation {} but composes with cogs in incompatible foundations \
             without an explicit functor-bridge.",
            shape.foundation.tag(),
        ),
        auto_fix_suggestion: Some(
            "Add a @framework(bridge_corpus, ...) declaration to translate between foundations, \
             or align the foundation across composing cogs."
                .into(),
        ),
    })
}

/// ATS-V-AP-006 — RegisterMixing.
/// Formal theorem cites authoritative-appeal / phenomenological /
/// traditional source.  CVE §6.7 L6 antiphilosophical invariant.
///
/// **Predicate**: `forall theorem.cited. ¬∃ ref ∈ cites. ref.kind ∈ {AuthoritativeAppeal, Phenomenological, Traditional}`.
pub fn check_register_mixing(
    shape: &Shape,
    forbidden_citations: &[ForbiddenCitation],
) -> Option<AntiPatternViolation> {
    let _ = shape;
    if forbidden_citations.is_empty() {
        return None;
    }
    let kinds: Vec<&str> = forbidden_citations.iter().map(|c| c.kind.tag()).collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::RegisterMixing,
        severity: Severity::Error,
        summary: format!(
            "Forbidden register citation(s): {}",
            kinds.join(", "),
        ),
        human_message: "Per CVE §6.7 (L6 antiphilosophical invariant), formal theorems must not cite \
                        authoritative-appeal, phenomenological, or traditional sources as justification."
            .to_string(),
        auto_fix_suggestion: Some(
            "Replace the forbidden register citation with a structural / kernel-discharged / formally-cited reference."
                .into(),
        ),
    })
}

#[derive(Debug, Clone)]
pub struct ForbiddenCitation {
    pub kind: ForbiddenRegisterKind,
    pub location: String,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForbiddenRegisterKind {
    AuthoritativeAppeal,
    Phenomenological,
    Traditional,
    Interpretive,
    OntologicalDeclaration,
}

impl ForbiddenRegisterKind {
    pub fn tag(&self) -> &'static str {
        match self {
            ForbiddenRegisterKind::AuthoritativeAppeal => "authoritative_appeal",
            ForbiddenRegisterKind::Phenomenological => "phenomenological",
            ForbiddenRegisterKind::Traditional => "traditional",
            ForbiddenRegisterKind::Interpretive => "interpretive",
            ForbiddenRegisterKind::OntologicalDeclaration => "ontological_declaration",
        }
    }
}

/// ATS-V-AP-007 — TxStraddling.
/// Transaction lives across an async/await boundary without scope.
///
/// **Predicate**: `forall tx : Affine. !crosses_async(tx)`.
pub fn check_tx_straddling(
    shape: &Shape,
    straddling_txs: &[String],
) -> Option<AntiPatternViolation> {
    let _ = shape;
    if straddling_txs.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::TxStraddling,
        severity: Severity::Error,
        summary: format!(
            "Transaction(s) straddle async boundaries: {}",
            straddling_txs.join(", "),
        ),
        human_message: "An affine transaction (held under @quantity(1)) outlives its async scope. \
                        Either commit/rollback before the await point, or restructure to keep the \
                        transaction scoped to a single async region."
            .to_string(),
        auto_fix_suggestion: Some(
            "Wrap the transaction-bearing block in `nursery { ... }` so structured concurrency \
             enforces scope."
                .into(),
        ),
    })
}

/// ATS-V-AP-008 — ResourceStraddling.
/// Linear resource (file handle, db connection) outlives its scope.
///
/// **Predicate**: `forall h : LinearResource. !escapes_scope(h)`.
pub fn check_resource_straddling(
    shape: &Shape,
    straddling_resources: &[String],
) -> Option<AntiPatternViolation> {
    let _ = shape;
    if straddling_resources.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::ResourceStraddling,
        severity: Severity::Error,
        summary: format!(
            "Linear resource(s) escape their scope: {}",
            straddling_resources.join(", "),
        ),
        human_message: "A linear resource (file handle, db connection) was returned/stored \
                        beyond its issuing scope. Linear resources must be released before \
                        their scope ends."
            .to_string(),
        auto_fix_suggestion: Some(
            "Use `defer resource.close()` to ensure scope-bound release, or restructure to \
             return ownership to the caller via @quantity(1)."
                .into(),
        ),
    })
}

/// ATS-V-AP-009 — LifecycleRegression.
/// `[Т]Theorem` cites `[Г]Hypothesis` / `[П]Plan` (transitive).
///
/// **Predicate**: `forall (citing, cited). citing.lifecycle.rank() ≤ cited.lifecycle.rank()`.
pub fn check_lifecycle_regression(
    shape: &Shape,
    cited_lifecycles: &[(String, Lifecycle)],
) -> Option<AntiPatternViolation> {
    let citing_rank = shape.lifecycle.rank();
    let regressions: Vec<(&str, &Lifecycle)> = cited_lifecycles
        .iter()
        .filter(|(_, cited)| citing_rank > cited.rank())
        .map(|(n, l)| (n.as_str(), l))
        .collect();
    if regressions.is_empty() {
        return None;
    }
    let names: Vec<String> = regressions
        .iter()
        .map(|(n, l)| format!("{} ({})", n, l.tag()))
        .collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::LifecycleRegression,
        severity: Severity::Error,
        summary: format!(
            "Lifecycle {} cog cites lower-rank: {}",
            shape.lifecycle.tag(),
            names.join(", "),
        ),
        human_message: format!(
            "A cog at lifecycle stage [{}] cites cogs at lower stages. \
             Higher-confidence artifacts must not depend on lower-confidence ones.",
            shape.lifecycle.tag(),
        ),
        auto_fix_suggestion: Some(
            "Either upgrade the cited cogs to a matching lifecycle stage, or downgrade this cog."
                .into(),
        ),
    })
}

/// ATS-V-AP-010 — CveIncomplete (strict mode only).
/// Public theorem without complete CVE-closure (all 3 axes).
///
/// **Predicate**: `forall public_theorem. cve_closure.is_fully_closed()`.
/// Active only when `@arch_module(strict = true)`.
pub fn check_cve_incomplete(shape: &Shape) -> Option<AntiPatternViolation> {
    if !shape.strict {
        return None;
    }
    if shape.cve_closure.is_fully_closed() {
        return None;
    }
    let mut missing = Vec::new();
    if shape.cve_closure.constructive.is_none() {
        missing.push("C (Constructive)");
    }
    if shape.cve_closure.verifiable_strategy.is_none() {
        missing.push("V (Verifiable)");
    }
    if shape.cve_closure.executable.is_none() {
        missing.push("E (Executable)");
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::CveIncomplete,
        severity: Severity::Error,
        summary: format!("CVE-closure incomplete: missing {}", missing.join(", ")),
        human_message: format!(
            "This cog is in strict mode but its CVE-closure triple is incomplete. Missing axes: {}. \
             Per CVE §2 + ATS-V §4.8, all three axes must be specified.",
            missing.join(", "),
        ),
        auto_fix_suggestion: Some(format!(
            "Add the missing axes to @arch_module: {}",
            missing
                .iter()
                .map(|m| match *m {
                    "C (Constructive)" => "cve_closure_C = <constructor_path>",
                    "V (Verifiable)" => "cve_closure_V_strategy = <strategy>",
                    "E (Executable)" => "cve_closure_E = <entry_point>",
                    _ => "",
                })
                .collect::<Vec<_>>()
                .join(", "),
        )),
    })
}

// =============================================================================
// Stratum admissibility — separate from main 10 anti-patterns
// =============================================================================

/// Separate check: `MsfsStratum::LAbs` is NOT a runtime-enforced
/// anti-pattern (it's structurally impossible per AFN-T α).
/// Checking it here as a sanity net: any cog that somehow ends
/// up declaring `LAbs` is flagged.  Reuses MsfsStratum's
/// `is_admissible()` predicate.
pub fn check_stratum_admissible(shape: &Shape) -> Option<AntiPatternViolation> {
    if shape.stratum.is_admissible() {
        return None;
    }
    // Re-use AP-005 FoundationDrift code for now since it shares the
    // semantic class "structurally impossible cross-stratum
    // composition"; future evolution may give LAbs its own code.
    Some(AntiPatternViolation {
        code: AntiPatternCode::FoundationDrift,
        severity: Severity::Error,
        summary: format!("Inadmissible MSFS stratum: {}", shape.stratum.tag()),
        human_message: "MSFS Theorem 5.1 (AFN-T α) proves L_Abs is empty. \
                        No cog can legitimately declare stratum = LAbs."
            .to_string(),
        auto_fix_suggestion: Some(
            "Choose stratum from {LFnd, LCls, LClsTop} per spec §4.7.".into(),
        ),
    })
}

// =============================================================================
// check_all — walk every check function over a Shape + diagnostic ctx
// =============================================================================

/// Diagnostic context — extra inputs the per-pattern checks need
/// beyond the Shape itself.  Populated by the ATS-V phase from
/// cog source analysis.
#[derive(Debug, Default)]
pub struct DiagnosticContext {
    pub cog_name: String,
    pub composes_graph: Vec<(String, Vec<String>)>,
    pub inferred_used_capabilities: Vec<Capability>,
    pub leaked_capabilities: Vec<Capability>,
    pub callee_tiers: Vec<(String, Tier)>,
    pub composed_foundations: Vec<(String, Foundation)>,
    pub forbidden_citations: Vec<ForbiddenCitation>,
    pub straddling_txs: Vec<String>,
    pub straddling_resources: Vec<String>,
    pub cited_lifecycles: Vec<(String, Lifecycle)>,
    // ----- MTAC fields (Сезон 5) -----
    /// Sample shapes at different time points для temporal
    /// inconsistency detection (AP-027).
    pub temporal_samples: Vec<(crate::arch_mtac::TimePoint, Shape)>,
    /// Counterfactual stability properties claimed by the cog
    /// (AP-028 CounterfactualBrittleness).
    pub counterfactual_pairs: Vec<crate::arch_mtac::CounterfactualPair>,
    /// Refactorings claimed на cog without adjoint pair
    /// (AP-029 MissedAdjoint).
    pub refactorings_without_adjoint: Vec<String>,
    /// Universal property claim для cog без uniqueness witness
    /// (AP-030 UniversalPropertyViolation).
    pub claimed_universal_property: Option<String>,
    pub uniqueness_witness: Option<String>,
    /// Evolution paths declared by cog с potentially-unsat
    /// triggers (AP-031 PhantomEvolution).
    pub declared_evolutions: Vec<crate::arch_mtac::ArchEvolution>,
    /// Refactoring claimed equivalent under Yoneda but
    /// observer-functor differs (AP-032 YonedaInequivalentRefactor).
    pub yoneda_observer_diff: Vec<(crate::arch_mtac::Observer, bool)>,
}

// =============================================================================
// MTAC anti-pattern checks (Сезон 5) — AP-027..032
// =============================================================================

/// ATS-V-AP-027 — TemporalInconsistency.
/// Cog at time t1 has invariant I, at t2 violates it.
///
/// **Predicate**: forall (t1, shape1), (t2, shape2) in temporal_samples.
/// shape1.foundation == shape2.foundation (foundation must be stable
/// across time).
pub fn check_temporal_inconsistency(
    _shape: &Shape,
    samples: &[(crate::arch_mtac::TimePoint, Shape)],
) -> Option<AntiPatternViolation> {
    if samples.len() < 2 {
        return None;
    }
    let first_foundation = &samples[0].1.foundation;
    let drift_at: Option<&crate::arch_mtac::TimePoint> = samples
        .iter()
        .skip(1)
        .find(|(_, s)| &s.foundation != first_foundation)
        .map(|(t, _)| t);
    if drift_at.is_none() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::TemporalInconsistency,
        severity: Severity::Error,
        summary: format!(
            "Foundation drifts across time samples (detected at {:?})",
            drift_at.unwrap().tag()
        ),
        human_message:
            "An MTAC `Always(Φ)` invariant requires the foundation to be stable across all time \
             points. This cog's temporal samples show foundation drift between time points."
                .to_string(),
        auto_fix_suggestion: Some(
            "Add an explicit @arch_corpus(foundation_bridge, ...) declaration if the foundation \
             must change, or align foundation across the temporal trajectory."
                .into(),
        ),
    })
}

/// ATS-V-AP-028 — CounterfactualBrittleness.
/// Cog works only under one decision; alternatives break stability invariants.
pub fn check_counterfactual_brittleness(
    _shape: &Shape,
    pairs: &[crate::arch_mtac::CounterfactualPair],
) -> Option<AntiPatternViolation> {
    let brittle: Vec<&crate::arch_mtac::CounterfactualPair> = pairs
        .iter()
        .filter(|p| p.stability_invariants.is_empty())
        .collect();
    if brittle.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::CounterfactualBrittleness,
        severity: Severity::Warning,
        summary: format!(
            "{} counterfactual pair(s) declared без stability invariants",
            brittle.len()
        ),
        human_message: "Counterfactual decision pairs claimed without explicit stability \
                        invariants — the cog may break under alternative decisions. Per spec §22.2 \
                        a stable counterfactual MUST declare which invariants hold across \
                        decision swaps."
            .to_string(),
        auto_fix_suggestion: Some(
            "Add stability_invariants list to each CounterfactualPair (e.g. PublicApiUnchanged)."
                .into(),
        ),
    })
}

/// ATS-V-AP-029 — MissedAdjoint.
/// Refactoring without inverse — one-way irreversible.
pub fn check_missed_adjoint(
    _shape: &Shape,
    refactorings_without_adjoint: &[String],
) -> Option<AntiPatternViolation> {
    if refactorings_without_adjoint.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::MissedAdjoint,
        severity: Severity::Warning,
        summary: format!(
            "{} refactoring(s) declared without adjoint pair",
            refactorings_without_adjoint.len()
        ),
        human_message: "Per spec §20.6, every refactoring is a pair (F, G) of functors with F ⊣ G \
                        (left adjoint). One-way irreversible refactorings violate this discipline."
            .to_string(),
        auto_fix_suggestion: Some(
            "Pair each refactoring with its adjoint counterpart, or mark explicitly as \
             irreversible via @arch_module(reversibility = Irreversible)."
                .into(),
        ),
    })
}

/// ATS-V-AP-030 — UniversalPropertyViolation.
/// Cog claims universal property без uniqueness witness.
pub fn check_universal_property_violation(
    _shape: &Shape,
    claimed: &Option<String>,
    witness: &Option<String>,
) -> Option<AntiPatternViolation> {
    if claimed.is_none() {
        return None; // no claim → no violation
    }
    if witness.is_some() {
        return None; // claim + witness → ok
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::UniversalPropertyViolation,
        severity: Severity::Error,
        summary: format!(
            "Universal property `{}` claimed без uniqueness witness",
            claimed.as_ref().unwrap()
        ),
        human_message:
            "Per spec §23, every claimed universal property must come with an explicit uniqueness \
             witness (no other cog provides the same property the same way). Without it, the cog \
             cannot be a Yoneda-canonical model."
                .to_string(),
        auto_fix_suggestion: Some(
            "Provide uniqueness_witness via @arch_module(universal_property = ..., \
             uniqueness_witness = ...)."
                .into(),
        ),
    })
}

/// ATS-V-AP-031 — PhantomEvolution.
/// Lifecycle declares evolution path с unsatisfiable trigger.
pub fn check_phantom_evolution(
    _shape: &Shape,
    evolutions: &[crate::arch_mtac::ArchEvolution],
) -> Option<AntiPatternViolation> {
    let phantoms: Vec<&crate::arch_mtac::ArchEvolution> = evolutions
        .iter()
        .filter(|e| e.trigger.is_empty() || e.trigger == "never")
        .collect();
    if phantoms.is_empty() {
        return None;
    }
    Some(AntiPatternViolation {
        code: AntiPatternCode::PhantomEvolution,
        severity: Severity::Warning,
        summary: format!(
            "{} declared evolution path(s) с unsatisfiable trigger",
            phantoms.len()
        ),
        human_message: "Per spec §21.3, ArchEvolution declarations must have satisfiable triggers. \
                        Empty / 'never' triggers indicate phantom evolutions that mislead readers \
                        and agent reasoning."
            .to_string(),
        auto_fix_suggestion: Some(
            "Either provide a concrete trigger condition or remove the evolution declaration."
                .into(),
        ),
    })
}

/// ATS-V-AP-032 — YonedaInequivalentRefactor.
/// Refactoring claimed equivalent но observer-functor changes.
pub fn check_yoneda_inequivalent_refactor(
    _shape: &Shape,
    observer_diff: &[(crate::arch_mtac::Observer, bool)],
) -> Option<AntiPatternViolation> {
    let mismatched: Vec<&(crate::arch_mtac::Observer, bool)> = observer_diff
        .iter()
        .filter(|(_, equivalent)| !*equivalent)
        .collect();
    if mismatched.is_empty() {
        return None;
    }
    let observer_tags: Vec<&str> = mismatched.iter().map(|(o, _)| o.tag()).collect();
    Some(AntiPatternViolation {
        code: AntiPatternCode::YonedaInequivalentRefactor,
        severity: Severity::Error,
        summary: format!(
            "Refactoring claims Yoneda equivalence но observer-functor differs for: {}",
            observer_tags.join(", ")
        ),
        human_message:
            "Per spec §20.7 + §23, two architectures are Yoneda-equivalent IFF they produce the \
             same observable behaviour for every observer. The refactoring changes behaviour for \
             at least one observer — equivalence claim is unsound."
                .to_string(),
        auto_fix_suggestion: Some(
            "Either correct the refactoring to preserve observer-functor equivalence, or downgrade \
             the equivalence claim to a weaker structural relation."
                .into(),
        ),
    })
}

/// Walk every canonical anti-pattern check; return all violations.
/// Used by ATS-V phase + audit gate.
pub fn check_all_anti_patterns(
    shape: &Shape,
    ctx: &DiagnosticContext,
) -> Vec<AntiPatternViolation> {
    let mut violations = Vec::new();
    if let Some(v) = check_capability_escalation(shape, &ctx.inferred_used_capabilities) {
        violations.push(v);
    }
    if let Some(v) = check_capability_leak(shape, &ctx.leaked_capabilities) {
        violations.push(v);
    }
    if let Some(v) = check_dependency_cycle(shape, &ctx.cog_name, &ctx.composes_graph) {
        violations.push(v);
    }
    if let Some(v) = check_tier_mixing(shape, &ctx.callee_tiers) {
        violations.push(v);
    }
    if let Some(v) = check_foundation_drift(shape, &ctx.composed_foundations) {
        violations.push(v);
    }
    if let Some(v) = check_register_mixing(shape, &ctx.forbidden_citations) {
        violations.push(v);
    }
    if let Some(v) = check_tx_straddling(shape, &ctx.straddling_txs) {
        violations.push(v);
    }
    if let Some(v) = check_resource_straddling(shape, &ctx.straddling_resources) {
        violations.push(v);
    }
    if let Some(v) = check_lifecycle_regression(shape, &ctx.cited_lifecycles) {
        violations.push(v);
    }
    if let Some(v) = check_cve_incomplete(shape) {
        violations.push(v);
    }
    if let Some(v) = check_stratum_admissible(shape) {
        violations.push(v);
    }
    // ----- MTAC checks (Сезон 5) — AP-027..032 -----
    if let Some(v) = check_temporal_inconsistency(shape, &ctx.temporal_samples) {
        violations.push(v);
    }
    if let Some(v) = check_counterfactual_brittleness(shape, &ctx.counterfactual_pairs) {
        violations.push(v);
    }
    if let Some(v) = check_missed_adjoint(shape, &ctx.refactorings_without_adjoint) {
        violations.push(v);
    }
    if let Some(v) = check_universal_property_violation(
        shape,
        &ctx.claimed_universal_property,
        &ctx.uniqueness_witness,
    ) {
        violations.push(v);
    }
    if let Some(v) = check_phantom_evolution(shape, &ctx.declared_evolutions) {
        violations.push(v);
    }
    if let Some(v) = check_yoneda_inequivalent_refactor(shape, &ctx.yoneda_observer_diff) {
        violations.push(v);
    }
    violations
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::*;

    #[test]
    fn anti_pattern_codes_are_distinct() {
        let probes = AntiPatternCode::full_list();
        let codes: std::collections::BTreeSet<_> = probes.iter().map(|c| c.code()).collect();
        assert_eq!(codes.len(), probes.len(), "every anti-pattern must have a distinct code");
        let names: std::collections::BTreeSet<_> = probes.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), probes.len());
    }

    #[test]
    fn anti_pattern_codes_have_stable_format() {
        for code in AntiPatternCode::full_list() {
            let c = code.code();
            assert!(c.starts_with("ATS-V-AP-"), "code must start with ATS-V-AP-: {}", c);
            assert!(c.len() == "ATS-V-AP-NNN".len(), "code length must match: {}", c);
        }
    }

    #[test]
    fn capability_escalation_detects_undeclared() {
        let mut shape = Shape::default_for_unannotated();
        shape.requires = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let inferred_used = vec![
            Capability::Read {
                resource: ResourceTag::Logger,
            }, // OK — declared
            Capability::Write {
                resource: ResourceTag::Logger,
            }, // VIOLATION — not declared
        ];
        let v = check_capability_escalation(&shape, &inferred_used);
        assert!(v.is_some());
        let v = v.unwrap();
        assert_eq!(v.code, AntiPatternCode::CapabilityEscalation);
    }

    #[test]
    fn capability_escalation_passes_when_all_declared() {
        let mut shape = Shape::default_for_unannotated();
        shape.requires = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let inferred = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        assert!(check_capability_escalation(&shape, &inferred).is_none());
    }

    #[test]
    fn dependency_cycle_detects_self_reference() {
        let shape = Shape::default_for_unannotated();
        let graph = vec![
            ("A".into(), vec!["B".into()]),
            ("B".into(), vec!["A".into()]),
        ];
        let v = check_dependency_cycle(&shape, "A", &graph);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::DependencyCycle);
    }

    #[test]
    fn dependency_cycle_passes_acyclic() {
        let shape = Shape::default_for_unannotated();
        let graph = vec![
            ("A".into(), vec!["B".into()]),
            ("B".into(), vec!["C".into()]),
            ("C".into(), vec![]),
        ];
        assert!(check_dependency_cycle(&shape, "A", &graph).is_none());
    }

    #[test]
    fn tier_mixing_detects_incompatible() {
        let mut shape = Shape::default_for_unannotated();
        shape.at_tier = Tier::Aot;
        let callees = vec![("gpu_fn".into(), Tier::Gpu)];
        let v = check_tier_mixing(&shape, &callees);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::TierMixing);
    }

    #[test]
    fn lifecycle_regression_detects_theorem_to_hypothesis() {
        let mut shape = Shape::default_for_unannotated();
        shape.lifecycle = Lifecycle::Theorem {
            since: "v0.1".into(),
        };
        let cited = vec![(
            "speculative_helper".into(),
            Lifecycle::Hypothesis {
                confidence: ConfidenceLevel::Low,
            },
        )];
        let v = check_lifecycle_regression(&shape, &cited);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::LifecycleRegression);
    }

    #[test]
    fn lifecycle_regression_passes_theorem_cites_theorem() {
        let mut shape = Shape::default_for_unannotated();
        shape.lifecycle = Lifecycle::Theorem {
            since: "v0.1".into(),
        };
        let cited = vec![(
            "stable_helper".into(),
            Lifecycle::Theorem {
                since: "v0.1".into(),
            },
        )];
        assert!(check_lifecycle_regression(&shape, &cited).is_none());
    }

    #[test]
    fn cve_incomplete_only_in_strict_mode() {
        let mut shape = Shape::default_for_unannotated();
        // Soft mode — no error even with empty CVE.
        assert!(check_cve_incomplete(&shape).is_none());
        // Strict mode — empty CVE is error.
        shape.strict = true;
        let v = check_cve_incomplete(&shape);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::CveIncomplete);
    }

    #[test]
    fn cve_complete_passes_in_strict_mode() {
        let mut shape = Shape::default_for_unannotated();
        shape.strict = true;
        shape.cve_closure = CveClosure {
            constructive: Some("c".into()),
            verifiable_strategy: Some(VerifyStrategy::Certified),
            executable: Some("e".into()),
        };
        assert!(check_cve_incomplete(&shape).is_none());
    }

    #[test]
    fn stratum_admissible_rejects_l_abs() {
        let mut shape = Shape::default_for_unannotated();
        shape.stratum = MsfsStratum::LAbs;
        let v = check_stratum_admissible(&shape);
        assert!(v.is_some());
    }

    #[test]
    fn stratum_admissible_passes_for_canonical_strata() {
        for stratum in [MsfsStratum::LFnd, MsfsStratum::LCls, MsfsStratum::LClsTop] {
            let mut shape = Shape::default_for_unannotated();
            shape.stratum = stratum;
            assert!(
                check_stratum_admissible(&shape).is_none(),
                "stratum {:?} must be admissible",
                stratum,
            );
        }
    }

    #[test]
    fn check_all_returns_empty_on_clean_default_shape() {
        let shape = Shape::default_for_unannotated();
        let ctx = DiagnosticContext::default();
        let violations = check_all_anti_patterns(&shape, &ctx);
        // Default-shape is trivially compliant.
        assert!(violations.is_empty(), "default shape must pass all anti-pattern checks");
    }

    #[test]
    fn violation_to_verdict_carries_code_and_docs_url() {
        // Pin: the canonical Verdict carrier preserves the anti-pattern
        // code + docs URL in metadata, so audit JSON can surface them
        // verbatim per spec §32.4.
        let v = AntiPatternViolation {
            code: AntiPatternCode::CapabilityEscalation,
            severity: Severity::Error,
            summary: "test".into(),
            human_message: "test".into(),
            auto_fix_suggestion: None,
        };
        let verdict = v.into_verdict();
        match verdict {
            crate::verdict::VerificationVerdict::Rejected { method, counterexample } => {
                match method {
                    crate::verdict::DischargeMethod::AtsVAntiPatternCheck { pattern_tag } => {
                        assert_eq!(pattern_tag, "CapabilityEscalation");
                    }
                    _ => panic!("expected AtsVAntiPatternCheck"),
                }
                assert_eq!(counterexample.metadata.get("code").map(String::as_str), Some("ATS-V-AP-001"));
                assert!(counterexample.metadata.contains_key("docs_url"));
            }
            _ => panic!("expected Rejected verdict"),
        }
    }

    #[test]
    fn architectural_pin_first_10_codes_reserved() {
        // Pin: the first 10 anti-patterns claim codes ATS-V-AP-001..010.
        // RFC stability — these never get re-used per spec §29.5.
        let codes: Vec<&str> = AntiPatternCode::full_list().iter().map(|c| c.code()).collect();
        assert_eq!(codes[0], "ATS-V-AP-001");
        assert_eq!(codes[9], "ATS-V-AP-010");
    }

    #[test]
    fn architectural_pin_32_total_codes_reserved() {
        // Сезон 2 catalog completion: 32 canonical patterns total.
        // Adding more requires RFC ATS-V-006 + community review per
        // spec §29.2.  Cap=30 in spec §7.1; current 32 includes
        // 6 MTAC patterns (§26) which are spec-introduced.
        let codes = AntiPatternCode::full_list();
        assert_eq!(codes.len(), 32);
        assert_eq!(codes[0].code(), "ATS-V-AP-001");
        assert_eq!(codes[31].code(), "ATS-V-AP-032");
    }

    #[test]
    fn season_attribution_correct() {
        // Pin: AP-001..010 are Season 1; AP-011..032 are Season 2.
        for (i, code) in AntiPatternCode::full_list().iter().enumerate() {
            let expected = if i < 10 { 1 } else { 2 };
            assert_eq!(
                code.season(),
                expected,
                "AP {} should be Season {}, got {}",
                code.code(),
                expected,
                code.season(),
            );
        }
    }

    #[test]
    fn mtac_patterns_recognised() {
        // Pin: 6 MTAC-specific patterns (AP-027..032) flagged
        // via is_mtac().  Used by audit gate JSON output to
        // separate base catalog from MTAC extensions.
        let mtac_count = AntiPatternCode::full_list()
            .iter()
            .filter(|c| c.is_mtac())
            .count();
        assert_eq!(mtac_count, 6);
        assert!(AntiPatternCode::TemporalInconsistency.is_mtac());
        assert!(AntiPatternCode::YonedaInequivalentRefactor.is_mtac());
        assert!(!AntiPatternCode::CapabilityEscalation.is_mtac());
    }

    #[test]
    fn all_codes_have_distinct_docs_urls() {
        // Pin: every code's docs_url() is distinct.  Catches off-by-one
        // bugs in the URL generation (AP-NNN format).
        let urls: std::collections::BTreeSet<_> = AntiPatternCode::full_list()
            .iter()
            .map(|c| c.docs_url())
            .collect();
        assert_eq!(urls.len(), 32);
        // Spot-check format.
        assert_eq!(
            AntiPatternCode::CapabilityEscalation.docs_url(),
            "https://verum.lang/docs/ats-v/ap-001"
        );
        assert_eq!(
            AntiPatternCode::YonedaInequivalentRefactor.docs_url(),
            "https://verum.lang/docs/ats-v/ap-032"
        );
    }

    // =========================================================================
    // MTAC checkers (Сезон 5) — direct unit pins
    // =========================================================================

    /// Helper: a Decision with name + no chosen value — sufficient
    /// for tests that only inspect the wrapper structure.
    fn dummy_decision(name: &str) -> crate::arch_mtac::Decision {
        crate::arch_mtac::Decision {
            name: name.to_string(),
            options: vec![],
            chosen: None,
            depends_on: vec![],
        }
    }

    #[test]
    fn temporal_inconsistency_detects_foundation_drift() {
        use crate::arch_mtac::TimePoint;
        let mut s_now = Shape::default_for_unannotated();
        s_now.foundation = Foundation::ZfcTwoInacc;
        let mut s_future = Shape::default_for_unannotated();
        s_future.foundation = Foundation::Hott;
        let samples = vec![
            (TimePoint::Now, s_now),
            (TimePoint::Future(2_000_000_000), s_future),
        ];
        let v = check_temporal_inconsistency(&Shape::default_for_unannotated(), &samples);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::TemporalInconsistency);
    }

    #[test]
    fn temporal_inconsistency_passes_stable_foundation() {
        use crate::arch_mtac::TimePoint;
        let s = Shape::default_for_unannotated();
        let samples = vec![
            (TimePoint::Now, s.clone()),
            (TimePoint::Future(2_000_000_000), s),
        ];
        assert!(
            check_temporal_inconsistency(&Shape::default_for_unannotated(), &samples).is_none()
        );
    }

    #[test]
    fn temporal_inconsistency_no_violation_with_single_sample() {
        use crate::arch_mtac::TimePoint;
        let samples = vec![(TimePoint::Now, Shape::default_for_unannotated())];
        assert!(
            check_temporal_inconsistency(&Shape::default_for_unannotated(), &samples).is_none()
        );
    }

    #[test]
    fn counterfactual_brittleness_detects_missing_invariants() {
        use crate::arch_mtac::CounterfactualPair;
        let pairs = vec![CounterfactualPair {
            name: "db_choice".into(),
            base: dummy_decision("use_pgsql"),
            alternative: dummy_decision("use_sqlite"),
            stability_invariants: vec![], // ← empty → brittle
        }];
        let v = check_counterfactual_brittleness(&Shape::default_for_unannotated(), &pairs);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::CounterfactualBrittleness);
    }

    #[test]
    fn counterfactual_brittleness_passes_with_invariants() {
        use crate::arch_mtac::{ArchProposition, CounterfactualPair};
        let pairs = vec![CounterfactualPair {
            name: "db_choice".into(),
            base: dummy_decision("use_pgsql"),
            alternative: dummy_decision("use_sqlite"),
            stability_invariants: vec![ArchProposition::PublicApiUnchanged],
        }];
        assert!(
            check_counterfactual_brittleness(&Shape::default_for_unannotated(), &pairs).is_none()
        );
    }

    #[test]
    fn missed_adjoint_detects_irreversible_refactor() {
        let refs = vec!["MergeModulesA_B".into()];
        let v = check_missed_adjoint(&Shape::default_for_unannotated(), &refs);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::MissedAdjoint);
    }

    #[test]
    fn missed_adjoint_passes_when_no_refactorings() {
        assert!(check_missed_adjoint(&Shape::default_for_unannotated(), &[]).is_none());
    }

    #[test]
    fn universal_property_violation_detects_claim_without_witness() {
        let v = check_universal_property_violation(
            &Shape::default_for_unannotated(),
            &Some("FreeMonoidOnX".into()),
            &None,
        );
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::UniversalPropertyViolation);
    }

    #[test]
    fn universal_property_violation_passes_with_witness() {
        let v = check_universal_property_violation(
            &Shape::default_for_unannotated(),
            &Some("FreeMonoidOnX".into()),
            &Some("uniqueness_proof_thm_4_2".into()),
        );
        assert!(v.is_none());
    }

    #[test]
    fn universal_property_violation_passes_no_claim() {
        let v = check_universal_property_violation(
            &Shape::default_for_unannotated(),
            &None,
            &None,
        );
        assert!(v.is_none());
    }

    #[test]
    fn phantom_evolution_detects_unsat_trigger() {
        use crate::arch_mtac::{
            ArchEvolution, ComplexityClass, Reversibility, TimePoint,
        };
        let evos = vec![ArchEvolution {
            trigger: "never".into(),
            expected_time: TimePoint::Future(2_000_000_000),
            cost_class: ComplexityClass::Linear,
            reversibility: Reversibility::Irreversible,
        }];
        let v = check_phantom_evolution(&Shape::default_for_unannotated(), &evos);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::PhantomEvolution);
    }

    #[test]
    fn phantom_evolution_passes_with_concrete_trigger() {
        use crate::arch_mtac::{
            ArchEvolution, ComplexityClass, Reversibility, TimePoint,
        };
        let evos = vec![ArchEvolution {
            trigger: "schema_v3 deployed".into(),
            expected_time: TimePoint::Future(2_000_000_000),
            cost_class: ComplexityClass::Linear,
            reversibility: Reversibility::AdjointReversible,
        }];
        assert!(check_phantom_evolution(&Shape::default_for_unannotated(), &evos).is_none());
    }

    #[test]
    fn yoneda_inequivalent_detects_observer_mismatch() {
        use crate::arch_mtac::Observer;
        let diff = vec![
            (
                Observer::EndUser {
                    kind: "default".into(),
                },
                true,
            ),
            (
                Observer::Auditor {
                    audit_kind: "compliance".into(),
                },
                false,
            ), // ← differs under refactor
        ];
        let v = check_yoneda_inequivalent_refactor(&Shape::default_for_unannotated(), &diff);
        assert!(v.is_some());
        assert_eq!(v.unwrap().code, AntiPatternCode::YonedaInequivalentRefactor);
    }

    #[test]
    fn yoneda_inequivalent_passes_when_all_observers_agree() {
        use crate::arch_mtac::Observer;
        let diff = vec![
            (
                Observer::EndUser {
                    kind: "default".into(),
                },
                true,
            ),
            (
                Observer::Auditor {
                    audit_kind: "compliance".into(),
                },
                true,
            ),
            (
                Observer::PeerCog {
                    module_path: "core::base".into(),
                },
                true,
            ),
        ];
        assert!(
            check_yoneda_inequivalent_refactor(&Shape::default_for_unannotated(), &diff).is_none()
        );
    }

    #[test]
    fn check_all_routes_through_mtac_checks() {
        // Pin: check_all_anti_patterns wires the 6 MTAC checks
        // — feed each violation context once and expect the
        // corresponding code to appear in the aggregated list.
        use crate::arch_mtac::{
            ArchEvolution, ComplexityClass, CounterfactualPair, Observer, Reversibility,
            TimePoint,
        };
        let shape = Shape::default_for_unannotated();
        let mut alt = Shape::default_for_unannotated();
        alt.foundation = Foundation::Hott;
        let ctx = DiagnosticContext {
            temporal_samples: vec![
                (TimePoint::Now, shape.clone()),
                (TimePoint::Future(2_000_000_000), alt),
            ],
            counterfactual_pairs: vec![CounterfactualPair {
                name: "any".into(),
                base: dummy_decision("a"),
                alternative: dummy_decision("b"),
                stability_invariants: vec![],
            }],
            refactorings_without_adjoint: vec!["one_way_merge".into()],
            claimed_universal_property: Some("FreeMonad".into()),
            uniqueness_witness: None,
            declared_evolutions: vec![ArchEvolution {
                trigger: String::new(),
                expected_time: TimePoint::Future(2_000_000_000),
                cost_class: ComplexityClass::Linear,
                reversibility: Reversibility::Irreversible,
            }],
            yoneda_observer_diff: vec![(
                Observer::Auditor {
                    audit_kind: "compliance".into(),
                },
                false,
            )],
            ..Default::default()
        };
        let violations = check_all_anti_patterns(&shape, &ctx);
        let codes: std::collections::HashSet<_> =
            violations.iter().map(|v| v.code).collect();
        // Each of the 6 MTAC codes must have surfaced.
        assert!(codes.contains(&AntiPatternCode::TemporalInconsistency));
        assert!(codes.contains(&AntiPatternCode::CounterfactualBrittleness));
        assert!(codes.contains(&AntiPatternCode::MissedAdjoint));
        assert!(codes.contains(&AntiPatternCode::UniversalPropertyViolation));
        assert!(codes.contains(&AntiPatternCode::PhantomEvolution));
        assert!(codes.contains(&AntiPatternCode::YonedaInequivalentRefactor));
    }
}
