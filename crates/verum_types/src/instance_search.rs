//! Phase D.4: Protocol Instance Search with Coherence Checking
//!
//! Automatic resolution of protocol instances ("trait instances" in
//! Rust terminology). When the type checker encounters a generic
//! function call that requires a protocol constraint like
//! `F: Monoid`, the instance search finds the concrete `implement
//! Monoid for F` block in the environment — or emits an error if no
//! implementation exists (or multiple conflicting ones do).
//!
//! ## Coherence
//!
//! Global coherence rule: for any given type `T` and protocol `P`,
//! there must be at most **one** `implement P for T` in the project.
//! Multiple implementations are a coherence violation — this module
//! detects the conflict and emits a diagnostic.
//!
//! The `@instance` attribute marks implementations as candidates for
//! automatic selection; the `@coherent` attribute asserts coherence
//! has been verified by the solver.

use verum_common::{List, Map, Text};

/// A protocol implementation candidate — a registered `implement P
/// for T` block that the search can return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceCandidate {
    /// The protocol name being implemented (e.g., `"Monoid"`).
    pub protocol: Text,
    /// The concrete type the instance is for (e.g., `"TwoFreeMonoid"`).
    pub target_type: Text,
    /// Type arguments to the protocol (for generic protocols).
    pub protocol_args: List<Text>,
    /// Unique identifier (module path + line number).
    pub source_location: Text,
    /// Has this candidate been marked `@instance`?
    pub is_instance_marked: bool,
    /// Has coherence been verified?
    pub is_coherent: bool,
}

impl InstanceCandidate {
    pub fn new(protocol: impl Into<Text>, target: impl Into<Text>) -> Self {
        Self {
            protocol: protocol.into(),
            target_type: target.into(),
            protocol_args: List::new(),
            source_location: Text::from(""),
            is_instance_marked: true, // by default, all implementations are candidates
            is_coherent: true,
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = Text>) -> Self {
        self.protocol_args = args.into_iter().collect();
        self
    }

    pub fn at(mut self, location: impl Into<Text>) -> Self {
        self.source_location = location.into();
        self
    }
}

/// The registry of all known protocol implementations.
///
/// Indexed by `(protocol, target_type)` for O(1) lookup; stores
/// a `Vec<InstanceCandidate>` per key to detect coherence violations.
#[derive(Debug, Default, Clone)]
pub struct InstanceRegistry {
    by_key: Map<(Text, Text), List<InstanceCandidate>>,
}

impl InstanceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new protocol implementation.
    pub fn register(&mut self, candidate: InstanceCandidate) {
        let key = (candidate.protocol.clone(), candidate.target_type.clone());
        self.by_key
            .entry(key)
            .or_insert_with(List::new)
            .push(candidate);
    }

    /// Count registered implementations.
    pub fn len(&self) -> usize {
        self.by_key.values().map(|v| v.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Find the unique protocol implementation for `(protocol,
    /// target_type)`.
    ///
    /// Returns:
    /// * `SearchResult::Unique(candidate)` — exactly one implementation.
    /// * `SearchResult::NotFound` — no `implement P for T` in the project.
    /// * `SearchResult::Ambiguous(candidates)` — multiple coherent
    ///   implementations (coherence violation).
    pub fn search(&self, protocol: &str, target: &str) -> SearchResult {
        let key = (Text::from(protocol), Text::from(target));
        match self.by_key.get(&key) {
            None => SearchResult::NotFound,
            Some(candidates) if candidates.is_empty() => SearchResult::NotFound,
            Some(candidates) if candidates.len() == 1 => {
                SearchResult::Unique(candidates[0].clone())
            }
            Some(candidates) => SearchResult::Ambiguous(candidates.clone()),
        }
    }

    /// Check global coherence: every `(protocol, target_type)` pair
    /// must have at most one implementation.
    pub fn check_coherence(&self) -> CoherenceReport {
        let mut violations = List::new();
        let mut total = 0usize;
        for (key, candidates) in &self.by_key {
            total += candidates.len();
            if candidates.len() > 1 {
                violations.push(CoherenceViolation {
                    protocol: key.0.clone(),
                    target_type: key.1.clone(),
                    conflicting_locations: candidates
                        .iter()
                        .map(|c| c.source_location.clone())
                        .collect(),
                });
            }
        }
        CoherenceReport {
            total_instances: total,
            violations,
        }
    }
}

/// Result of an instance-search query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchResult {
    Unique(InstanceCandidate),
    NotFound,
    Ambiguous(List<InstanceCandidate>),
}

/// A detected coherence violation — two or more implementations of
/// the same protocol for the same target type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceViolation {
    pub protocol: Text,
    pub target_type: Text,
    pub conflicting_locations: List<Text>,
}

/// Summary of coherence checking across the whole registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceReport {
    pub total_instances: usize,
    pub violations: List<CoherenceViolation>,
}

impl CoherenceReport {
    pub fn is_coherent(&self) -> bool {
        self.violations.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(p: &str, t: &str, loc: &str) -> InstanceCandidate {
        InstanceCandidate::new(p, t).at(loc)
    }

    #[test]
    fn test_empty_registry_not_found() {
        let reg = InstanceRegistry::new();
        assert_eq!(reg.search("Monoid", "T"), SearchResult::NotFound);
    }

    #[test]
    fn test_single_instance_found() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "Z3", "core/math/examples.vr:230"));
        match reg.search("Monoid", "Z3") {
            SearchResult::Unique(c) => {
                assert_eq!(c.protocol.as_str(), "Monoid");
                assert_eq!(c.target_type.as_str(), "Z3");
            }
            other => panic!("expected Unique, got {:?}", other),
        }
    }

    #[test]
    fn test_ambiguous_instances() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "T", "loc1"));
        reg.register(candidate("Monoid", "T", "loc2"));
        match reg.search("Monoid", "T") {
            SearchResult::Ambiguous(cs) => assert_eq!(cs.len(), 2),
            other => panic!("expected Ambiguous, got {:?}", other),
        }
    }

    #[test]
    fn test_coherence_empty_is_coherent() {
        let reg = InstanceRegistry::new();
        let report = reg.check_coherence();
        assert!(report.is_coherent());
        assert_eq!(report.total_instances, 0);
    }

    #[test]
    fn test_coherence_single_instance_is_coherent() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "Z3", "loc1"));
        let report = reg.check_coherence();
        assert!(report.is_coherent());
        assert_eq!(report.total_instances, 1);
    }

    #[test]
    fn test_coherence_detects_duplicates() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "T", "loc1"));
        reg.register(candidate("Monoid", "T", "loc2"));
        let report = reg.check_coherence();
        assert!(!report.is_coherent());
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].protocol.as_str(), "Monoid");
        assert_eq!(report.violations[0].target_type.as_str(), "T");
        assert_eq!(report.violations[0].conflicting_locations.len(), 2);
    }

    #[test]
    fn test_coherence_allows_different_targets() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "Z3", "loc1"));
        reg.register(candidate("Monoid", "Nat4", "loc2"));
        reg.register(candidate("Monoid", "F2", "loc3"));
        let report = reg.check_coherence();
        assert!(report.is_coherent());
        assert_eq!(report.total_instances, 3);
    }

    #[test]
    fn test_coherence_allows_different_protocols_same_target() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "Z3", "loc1"));
        reg.register(candidate("Group", "Z3", "loc2"));
        reg.register(candidate("AbelianGroup", "Z3", "loc3"));
        let report = reg.check_coherence();
        assert!(report.is_coherent());
        assert_eq!(report.total_instances, 3);
    }

    #[test]
    fn test_registry_len_counts_all_candidates() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Monoid", "Z3", "loc1"));
        reg.register(candidate("Group", "Z3", "loc2"));
        reg.register(candidate("Monoid", "Nat4", "loc3"));
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn test_candidate_with_protocol_args() {
        let c = InstanceCandidate::new("Category", "IntegerPathCategory")
            .with_args([Text::from("Int"), Text::from("PathInt")])
            .at("core/math/examples.vr:100");
        assert_eq!(c.protocol_args.len(), 2);
        assert_eq!(c.protocol_args[0].as_str(), "Int");
    }
}
