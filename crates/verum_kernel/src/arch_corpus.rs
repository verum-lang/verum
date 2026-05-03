//! ATS-V cross-cog corpus invariants — `@arch_corpus(...)`.
//!
//! ## Architectural role
//!
//! Per `internal/specs/ats-v.md` §11.3 ( deliverable) +
//! §16 RFC ATS-V-001, individual `@arch_module(...)` declarations
//! cover per-cog invariants. Cross-cog invariants — properties
//! that hold over the entire corpus, not any single cog —
//! require a separate scope: `@arch_corpus(...)`.
//!
//! ## Canonical corpus invariants
//!
//! ships 4 baseline cross-cog invariants:
//!
//! 1. **NoCircularDependencies** — composition graph acyclic
//! (transitive across all cogs).
//! 2. **FoundationConsistency** — all cogs share a foundation,
//! OR pairs with different foundations have explicit
//! `@framework(bridge_corpus, ...)` declarations.
//! 3. **NoLAbsClaim** — no cog declares `stratum = LAbs`
//! (sanity net for the AFN-T α boundary closure; AP-011
//! handles per-cog).
//! 4. **CapabilityClosure** — for every cog A in corpus, every
//! capability A.requires either has a producer cog (some B
//! with that capability в B.exposes) OR is documented as
//! "external" (capability registered as `transfers_privilege:
//! true` in capability_ontology).
//!
//! ## Reuse over invention
//!
//! The corpus invariants are **derived** from per-cog Shape data
//! computed by `arch_phase::run_arch_phase`. No new parser, no
//! new attribute types — `@arch_corpus(...)` is just another
//! typed attribute с named-args specifying which invariants to
//! verify and any cog-specific overrides.

use crate::arch::{Capability, Foundation, MsfsStratum, Shape};
use std::collections::{BTreeMap, BTreeSet};

// =============================================================================
// CorpusInvariant — kinds of cross-cog invariants
// =============================================================================

/// Stable enumeration of corpus-level invariants. Each variant
/// has a check function returning structured violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CorpusInvariant {
    /// No circular dependencies in the cross-cog mount graph (AP-003).
    NoCircularDependencies,
    /// Foundations of composed cogs match or have a registered bridge (AP-005).
    FoundationConsistency,
    /// No cog claims `MsfsStratum::LAbs` (AP-011 — AFN-T α violation).
    NoLAbsClaim,
    /// Every required capability is exposed by some cog in the mount graph.
    CapabilityClosure,
}

impl CorpusInvariant {
 /// Stable diagnostic tag.
    pub fn tag(&self) -> &'static str {
        match self {
            CorpusInvariant::NoCircularDependencies => "no_circular_dependencies",
            CorpusInvariant::FoundationConsistency => "foundation_consistency",
            CorpusInvariant::NoLAbsClaim => "no_l_abs_claim",
            CorpusInvariant::CapabilityClosure => "capability_closure",
        }
    }

 /// Human-friendly name.
    pub fn name(&self) -> &'static str {
        match self {
            CorpusInvariant::NoCircularDependencies => "NoCircularDependencies",
            CorpusInvariant::FoundationConsistency => "FoundationConsistency",
            CorpusInvariant::NoLAbsClaim => "NoLAbsClaim",
            CorpusInvariant::CapabilityClosure => "CapabilityClosure",
        }
    }

 /// Full canonical list — baseline.
    pub fn full_list() -> [CorpusInvariant; 4] {
        [
            CorpusInvariant::NoCircularDependencies,
            CorpusInvariant::FoundationConsistency,
            CorpusInvariant::NoLAbsClaim,
            CorpusInvariant::CapabilityClosure,
        ]
    }
}

// =============================================================================
// CorpusViolation — structured diagnostic
// =============================================================================

/// Structured diagnostic produced when a corpus invariant fails.
#[derive(Debug, Clone)]
pub struct CorpusViolation {
 /// Which invariant was violated.
    pub invariant: CorpusInvariant,
 /// One-line summary.
    pub summary: String,
 /// Human-friendly message.
    pub human_message: String,
 /// Affected cog(s).
    pub affected_cogs: Vec<String>,
}

// =============================================================================
// CorpusReport — aggregate report
// =============================================================================

/// Aggregated cross-cog verification report.
#[derive(Debug, Clone, Default)]
pub struct CorpusReport {
    /// Total number of cogs the corpus walker visited.
    pub total_cogs: usize,
    /// Per-violation diagnostics surfaced during the walk.
    pub violations: Vec<CorpusViolation>,
}

impl CorpusReport {
 /// True iff no corpus invariant violated.
    pub fn is_load_bearing(&self) -> bool {
        self.violations.is_empty()
    }

 /// Group violations by invariant.
    pub fn by_invariant(&self) -> BTreeMap<&'static str, usize> {
        let mut by = BTreeMap::new();
        for v in &self.violations {
            *by.entry(v.invariant.tag()).or_insert(0) += 1;
        }
        by
    }
}

// =============================================================================
// verify_corpus — main entry point
// =============================================================================

/// Verify cross-cog invariants over a corpus.
///
/// `corpus` is a slice of (cog_name, Shape) pairs. The function
/// runs all 4 baseline invariants and returns a `CorpusReport`
/// with structured violations.
pub fn verify_corpus(corpus: &[(String, Shape)]) -> CorpusReport {
    let mut report = CorpusReport {
        total_cogs: corpus.len(),
        violations: Vec::new(),
    };

 // Invariant 1: NoCircularDependencies.
    if let Some(v) = check_no_circular_dependencies(corpus) {
        report.violations.push(v);
    }
 // Invariant 2: FoundationConsistency.
    if let Some(v) = check_foundation_consistency(corpus) {
        report.violations.push(v);
    }
 // Invariant 3: NoLAbsClaim.
    if let Some(v) = check_no_l_abs_claim(corpus) {
        report.violations.push(v);
    }
 // Invariant 4: CapabilityClosure.
    if let Some(v) = check_capability_closure(corpus) {
        report.violations.push(v);
    }

    report
}

// =============================================================================
// Per-invariant checkers
// =============================================================================

/// Check that the composition graph (composes_with) is acyclic
/// across all cogs.
pub fn check_no_circular_dependencies(corpus: &[(String, Shape)]) -> Option<CorpusViolation> {
 // Build graph: name -> [composes_with].
    let edges: BTreeMap<&str, &[String]> = corpus
        .iter()
        .map(|(n, s)| (n.as_str(), s.composes_with.as_slice()))
        .collect();

    let mut affected: BTreeSet<String> = BTreeSet::new();
    for (name, _) in corpus {
        if has_cycle_from(name, &edges) {
            affected.insert(name.clone());
        }
    }
    if affected.is_empty() {
        return None;
    }
    let affected_vec: Vec<String> = affected.into_iter().collect();
    Some(CorpusViolation {
        invariant: CorpusInvariant::NoCircularDependencies,
        summary: format!(
            "{} cog(s) participate in a composition cycle",
            affected_vec.len()
        ),
        human_message: "The corpus contains a cycle in @arch_module(composes_with) declarations. \
                        Architectural composition graphs must be acyclic.".to_string(),
        affected_cogs: affected_vec,
    })
}

fn has_cycle_from(start: &str, edges: &BTreeMap<&str, &[String]>) -> bool {
    let mut visiting: BTreeSet<&str> = BTreeSet::new();
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    fn dfs<'a>(
        node: &'a str,
        edges: &BTreeMap<&'a str, &'a [String]>,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
    ) -> bool {
        if visiting.contains(node) {
            return true;
        }
        if visited.contains(node) {
            return false;
        }
        visiting.insert(node);
        if let Some(neighbours) = edges.get(node) {
            for n in *neighbours {
                if dfs(n.as_str(), edges, visiting, visited) {
                    return true;
                }
            }
        }
        visiting.remove(node);
        visited.insert(node);
        false
    }
    dfs(start, edges, &mut visiting, &mut visited)
}

/// Check foundation consistency: all cogs share foundation OR
/// pairs with different foundations have an explicit bridge.
/// simplification: we check shared-foundation only;
/// bridge declarations land with framework_translate
/// integration.
pub fn check_foundation_consistency(corpus: &[(String, Shape)]) -> Option<CorpusViolation> {
    if corpus.is_empty() {
        return None;
    }
    let mut foundations: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, shape) in corpus {
        foundations
            .entry(shape.foundation.tag().to_string())
            .or_default()
            .push(name.clone());
    }
    if foundations.len() <= 1 {
        return None; // single foundation across corpus
    }
 // Multiple foundations — check pairwise direct subsumption.
    let unique: Vec<&Foundation> = corpus
        .iter()
        .map(|(_, s)| &s.foundation)
        .collect::<Vec<_>>()
        .into_iter()
        .fold(Vec::new(), |mut acc, f| {
            if !acc.iter().any(|x: &&Foundation| *x == f) {
                acc.push(f);
            }
            acc
        });
    let mut all_compatible = true;
    for (i, a) in unique.iter().enumerate() {
        for b in unique.iter().skip(i + 1) {
            if !a.directly_subsumed_by(b) && !b.directly_subsumed_by(a) {
                all_compatible = false;
                break;
            }
        }
    }
    if all_compatible {
        return None;
    }
    let foundation_names: Vec<String> = foundations.keys().cloned().collect();
    let affected: Vec<String> = foundations.values().flatten().cloned().collect();
    Some(CorpusViolation {
        invariant: CorpusInvariant::FoundationConsistency,
        summary: format!(
            "Corpus mixes incompatible foundations: {}",
            foundation_names.join(", ")
        ),
        human_message: "Multiple cogs declare incompatible foundations without explicit \
                        functor-bridges. Either align foundations across the corpus, or \
                        add @framework(bridge_corpus, ...) declarations for cross-paradigm \
                        translation.".to_string(),
        affected_cogs: affected,
    })
}

/// Check that no cog declares `stratum = LAbs` (sanity net for
/// AFN-T α boundary; AP-011 handles per-cog).
pub fn check_no_l_abs_claim(corpus: &[(String, Shape)]) -> Option<CorpusViolation> {
    let affected: Vec<String> = corpus
        .iter()
        .filter(|(_, s)| matches!(s.stratum, MsfsStratum::LAbs))
        .map(|(n, _)| n.clone())
        .collect();
    if affected.is_empty() {
        return None;
    }
    Some(CorpusViolation {
        invariant: CorpusInvariant::NoLAbsClaim,
        summary: format!(
            "{} cog(s) declare inadmissible stratum LAbs",
            affected.len()
        ),
        human_message: "MSFS Theorem 5.1 (AFN-T α — Boundary Lemma) proves L_Abs is empty. \
                        No cog can legitimately declare stratum = LAbs.".to_string(),
        affected_cogs: affected,
    })
}

/// Check capability closure: every required capability has a
/// producer in corpus, OR is registered as external (privilege-
/// transferring) in capability_ontology.
pub fn check_capability_closure(corpus: &[(String, Shape)]) -> Option<CorpusViolation> {
 // Collect all exposed capabilities (their tag strings) keyed
 // by producer cog.
    let mut producers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cog_name, shape) in corpus {
        for cap in &shape.exposes {
            producers
                .entry(capability_id(cap))
                .or_default()
                .push(cog_name.clone());
        }
    }

 // Find required-but-unproduced.
    let mut affected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (cog_name, shape) in corpus {
        for cap in &shape.requires {
            let id = capability_id(cap);
            if !producers.contains_key(&id) && !is_externally_registered(cap) {
                affected
                    .entry(cog_name.clone())
                    .or_default()
                    .push(id);
            }
        }
    }
    if affected.is_empty() {
        return None;
    }
    let affected_cogs: Vec<String> = affected.keys().cloned().collect();
    let summary_parts: Vec<String> = affected
        .iter()
        .map(|(cog, caps)| format!("{} (needs: {})", cog, caps.join(",")))
        .collect();
    Some(CorpusViolation {
        invariant: CorpusInvariant::CapabilityClosure,
        summary: format!(
            "Corpus capability closure incomplete: {} cog(s) require unproduced capabilities",
            affected_cogs.len()
        ),
        human_message: format!(
            "The following cogs require capabilities that are neither exposed by any cog \
             in the corpus nor registered as external in capability_ontology.vr: {}",
            summary_parts.join("; ")
        ),
        affected_cogs,
    })
}

/// Build a stable identifier string for a Capability, used as the
/// dictionary key in capability closure checks.
fn capability_id(cap: &Capability) -> String {
    match cap {
        Capability::Custom { tag, .. } => format!("custom:{}", tag),
        Capability::Read { resource } => format!("read:{:?}", resource),
        Capability::Write { resource } => format!("write:{:?}", resource),
        Capability::Exec { target } => format!("exec:{:?}", target),
        Capability::Escalate { realm } => format!("escalate:{:?}", realm),
        Capability::Spawn { lifetime } => format!("spawn:{:?}", lifetime),
        Capability::TimeBound { until } => format!("time_bound:{:?}", until),
        Capability::Persist { medium } => format!("persist:{:?}", medium),
        Capability::Network {
            protocol,
            direction,
        } => format!("network:{:?}/{:?}", protocol, direction),
    }
}

/// Stub: capability_ontology.vr resolution. 
/// reads the actual ontology file and returns true for registered
/// names. For now, conservatively treat custom capabilities
/// matching well-known ontology names as external.
fn is_externally_registered(cap: &Capability) -> bool {
    let well_known_external = [
        "logger",
        "metrics",
        "tracing",
        "config_read",
        "config_admin",
        "supervisor_spawn",
        "kernel_intrinsic",
    ];
    matches!(cap, Capability::Custom { tag, .. } if well_known_external.contains(&tag.as_str()))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::*;

    fn make_shape(
        composes_with: Vec<String>,
        exposes: Vec<Capability>,
        requires: Vec<Capability>,
    ) -> Shape {
        let mut s = Shape::default_for_unannotated();
        s.composes_with = composes_with;
        s.exposes = exposes;
        s.requires = requires;
        s
    }

    fn cap(tag: &str) -> Capability {
        Capability::Custom {
            tag: tag.to_string(),
            schema: CapabilitySchema {
                description: "test".into(),
                transfers_privilege: false,
                subsumed_by: vec![],
            },
        }
    }

    #[test]
    fn corpus_invariant_tags_distinct() {
        let probes = CorpusInvariant::full_list();
        let tags: std::collections::BTreeSet<_> = probes.iter().map(|i| i.tag()).collect();
        assert_eq!(tags.len(), 4);
    }

    #[test]
    fn empty_corpus_passes_all_invariants() {
        let report = verify_corpus(&[]);
        assert!(report.is_load_bearing());
        assert_eq!(report.total_cogs, 0);
    }

    #[test]
    fn single_cog_corpus_passes() {
        let corpus = vec![("solo".to_string(), Shape::default_for_unannotated())];
        let report = verify_corpus(&corpus);
        assert!(report.is_load_bearing());
        assert_eq!(report.total_cogs, 1);
    }

    #[test]
    fn circular_dependency_detected() {
        let corpus = vec![
            ("A".to_string(), make_shape(vec!["B".into()], vec![], vec![])),
            ("B".to_string(), make_shape(vec!["A".into()], vec![], vec![])),
        ];
        let report = verify_corpus(&corpus);
        assert!(!report.is_load_bearing());
        assert!(report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::NoCircularDependencies));
    }

    #[test]
    fn acyclic_chain_passes() {
        let corpus = vec![
            ("A".to_string(), make_shape(vec!["B".into()], vec![], vec![])),
            ("B".to_string(), make_shape(vec!["C".into()], vec![], vec![])),
            ("C".to_string(), make_shape(vec![], vec![], vec![])),
        ];
        let report = verify_corpus(&corpus);
        assert!(!report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::NoCircularDependencies));
    }

    #[test]
    fn l_abs_claim_detected() {
        let mut bad = Shape::default_for_unannotated();
        bad.stratum = MsfsStratum::LAbs;
        let corpus = vec![("escape_attempt".to_string(), bad)];
        let report = verify_corpus(&corpus);
        assert!(!report.is_load_bearing());
        assert!(report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::NoLAbsClaim));
    }

    #[test]
    fn foundation_consistency_passes_single_foundation() {
        let corpus = vec![
            ("A".to_string(), Shape::default_for_unannotated()),
            ("B".to_string(), Shape::default_for_unannotated()),
        ];
        let report = verify_corpus(&corpus);
        assert!(!report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::FoundationConsistency));
    }

    #[test]
    fn foundation_consistency_passes_canonical_subsumption() {
 // CIC ⊃ MLTT — directly subsumed.
        let mut a = Shape::default_for_unannotated();
        a.foundation = Foundation::Cic;
        let mut b = Shape::default_for_unannotated();
        b.foundation = Foundation::Mltt;
        let corpus = vec![("A".to_string(), a), ("B".to_string(), b)];
        let report = verify_corpus(&corpus);
        assert!(!report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::FoundationConsistency));
    }

    #[test]
    fn foundation_consistency_detects_incompatible() {
        let mut a = Shape::default_for_unannotated();
        a.foundation = Foundation::ZfcTwoInacc;
        let mut b = Shape::default_for_unannotated();
        b.foundation = Foundation::Hott;
        let corpus = vec![("A".to_string(), a), ("B".to_string(), b)];
        let report = verify_corpus(&corpus);
        assert!(report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::FoundationConsistency));
    }

    #[test]
    fn capability_closure_passes_when_satisfied() {
 // A exposes [logger]; B requires [logger].
        let corpus = vec![
            ("A".to_string(), make_shape(vec![], vec![cap("logger")], vec![])),
            ("B".to_string(), make_shape(vec![], vec![], vec![cap("logger")])),
        ];
        let report = verify_corpus(&corpus);
        assert!(!report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::CapabilityClosure));
    }

    #[test]
    fn capability_closure_passes_for_externally_registered() {
 // B requires [logger] but no one exposes it; logger is
 // in capability_ontology baseline → externally registered
 // → no violation.
        let corpus = vec![("B".to_string(), make_shape(vec![], vec![], vec![cap("logger")]))];
        let report = verify_corpus(&corpus);
        assert!(!report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::CapabilityClosure));
    }

    #[test]
    fn capability_closure_detects_missing_producer() {
 // B requires [unknown_capability] which no one produces
 // and isn't in baseline ontology.
        let corpus = vec![(
            "B".to_string(),
            make_shape(vec![], vec![], vec![cap("totally_unknown")]),
        )];
        let report = verify_corpus(&corpus);
        assert!(report
            .violations
            .iter()
            .any(|v| v.invariant == CorpusInvariant::CapabilityClosure));
    }

    #[test]
    fn architectural_pin_4_invariants_in_canonical_list() {
 // baseline = exactly 4 corpus invariants.
 // Adding more requires RFC ATS-V-001 (per spec §16).
        assert_eq!(CorpusInvariant::full_list().len(), 4);
    }

    #[test]
    fn report_groups_violations_by_invariant() {
        let mut bad1 = Shape::default_for_unannotated();
        bad1.stratum = MsfsStratum::LAbs;
        let mut bad2 = Shape::default_for_unannotated();
        bad2.stratum = MsfsStratum::LAbs;
        let corpus = vec![("A".to_string(), bad1), ("B".to_string(), bad2)];
        let report = verify_corpus(&corpus);
        let by = report.by_invariant();
 // Both LAbs trigger ONE composite violation
 // (multi-affected).
        assert!(by.contains_key("no_l_abs_claim"));
    }
}
