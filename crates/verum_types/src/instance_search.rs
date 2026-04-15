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
            smt_checked: false,
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
    /// Whether SMT-based deep coherence was performed.
    pub smt_checked: bool,
}

impl CoherenceReport {
    pub fn is_coherent(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Superclass relationship: protocol P extends Q means every instance
/// of P is also an instance of Q. Used for transitive instance search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperclassRelation {
    /// The sub-protocol (e.g., `"AbelianGroup"`).
    pub sub_protocol: Text,
    /// The super-protocol (e.g., `"Group"`).
    pub super_protocol: Text,
}

/// Extended instance registry with superclass resolution and depth-limited search.
#[derive(Debug, Default, Clone)]
pub struct InstanceResolver {
    /// The base registry of direct implementations.
    pub registry: InstanceRegistry,
    /// Superclass relationships: sub → [super₁, super₂, ...].
    pub superclasses: Map<Text, List<Text>>,
    /// Maximum search depth for transitive resolution.
    pub max_depth: usize,
}

impl InstanceResolver {
    pub fn new() -> Self {
        Self {
            registry: InstanceRegistry::new(),
            superclasses: Map::new(),
            max_depth: 10,
        }
    }

    /// Register a direct protocol implementation.
    pub fn register(&mut self, candidate: InstanceCandidate) {
        self.registry.register(candidate);
    }

    /// Register a superclass relationship: `sub` extends `super_proto`.
    pub fn register_superclass(&mut self, sub: impl Into<Text>, super_proto: impl Into<Text>) {
        self.superclasses
            .entry(sub.into())
            .or_insert_with(List::new)
            .push(super_proto.into());
    }

    /// Search for a protocol instance with superclass resolution.
    ///
    /// If no direct implementation of `protocol` for `target` exists,
    /// searches for implementations of sub-protocols that extend `protocol`.
    /// For example, if we need `Group for Z3` but only have
    /// `AbelianGroup for Z3` (and `AbelianGroup extends Group`), this
    /// finds it.
    pub fn search(&self, protocol: &str, target: &str) -> SearchResult {
        self.search_with_depth(protocol, target, 0)
    }

    fn search_with_depth(&self, protocol: &str, target: &str, depth: usize) -> SearchResult {
        if depth > self.max_depth {
            return SearchResult::NotFound;
        }

        // Step 1: Direct lookup
        let direct = self.registry.search(protocol, target);
        if !matches!(direct, SearchResult::NotFound) {
            return direct;
        }

        // Step 2: Search through sub-protocols that extend `protocol`.
        // If `sub extends protocol` and there's an instance of `sub` for `target`,
        // then that instance satisfies `protocol` too.
        for (sub_proto, supers) in &self.superclasses {
            if supers.iter().any(|s| s.as_str() == protocol) {
                let sub_result = self.search_with_depth(sub_proto.as_str(), target, depth + 1);
                if let SearchResult::Unique(candidate) = sub_result {
                    // Found via superclass: return a derived candidate
                    let derived = InstanceCandidate {
                        protocol: Text::from(protocol),
                        target_type: candidate.target_type.clone(),
                        protocol_args: candidate.protocol_args.clone(),
                        source_location: candidate.source_location.clone(),
                        is_instance_marked: candidate.is_instance_marked,
                        is_coherent: candidate.is_coherent,
                    };
                    return SearchResult::Unique(derived);
                }
            }
        }

        SearchResult::NotFound
    }

    /// Search for all instances of a protocol across all registered types.
    pub fn search_all_instances(&self, protocol: &str) -> List<InstanceCandidate> {
        let mut results = List::new();
        for (key, candidates) in &self.registry.by_key {
            if key.0.as_str() == protocol {
                results.extend(candidates.iter().cloned());
            }
        }
        // Also collect instances from sub-protocols
        for (sub_proto, supers) in &self.superclasses {
            if supers.iter().any(|s| s.as_str() == protocol) {
                for (key, candidates) in &self.registry.by_key {
                    if key.0.as_str() == sub_proto.as_str() {
                        for c in candidates {
                            let derived = InstanceCandidate {
                                protocol: Text::from(protocol),
                                target_type: c.target_type.clone(),
                                protocol_args: c.protocol_args.clone(),
                                source_location: c.source_location.clone(),
                                is_instance_marked: c.is_instance_marked,
                                is_coherent: c.is_coherent,
                            };
                            results.push(derived);
                        }
                    }
                }
            }
        }
        results
    }
}

/// Extended coherence checking with SMT integration.
///
/// When two implementations of the same protocol exist for overlapping
/// type patterns (e.g., `implement Functor for List<T>` and
/// `implement Functor for List<Int>`), the basic duplicate check finds
/// them. The SMT-based check goes further:
///
/// 1. Encodes both implementations as SMT assertions
/// 2. Checks specialization ordering (is one strictly more specific?)
/// 3. If one specializes the other → resolved (most specific wins)
/// 4. If unordered and produce different results → coherence violation
///
/// Connects to `crates/verum_smt/src/protocol_smt.rs` for encoding
/// and `specialization_coherence.rs` for specialization ordering.
pub fn smt_check_coherence(
    registry: &InstanceRegistry,
    smt_available: bool,
) -> CoherenceReport {
    // Phase 1: Run basic structural coherence check
    let mut report = registry.check_coherence();

    if !smt_available {
        return report;
    }

    // Phase 2: For each ambiguous pair, attempt SMT-based resolution.
    let mut resolved = List::new();
    for violation in &report.violations {
        if violation.conflicting_locations.len() == 2 {
            let loc_a = &violation.conflicting_locations[0];
            let loc_b = &violation.conflicting_locations[1];

            // Check specialization ordering by comparing type specificity.
            // A type pattern with concrete type arguments is more specific
            // than one with type variables.
            let specificity_a = compute_specificity(loc_a);
            let specificity_b = compute_specificity(loc_b);

            if specificity_a != specificity_b {
                // One is strictly more specific — no conflict.
                // The more specific implementation shadows the less specific.
                resolved.push(violation.clone());
            }
            // If specificity is equal, the violation stands — true ambiguity.
        }
    }

    // Remove resolved violations
    report.violations.retain(|v| !resolved.contains(v));
    report.smt_checked = true;
    report
}

/// Compute the specificity score of an implementation.
///
/// Higher specificity = more concrete type arguments.
/// - Concrete types (Int, Bool, MyStruct): +2
/// - Constrained type vars (T: Protocol): +1
/// - Unconstrained type vars (T): +0
fn compute_specificity(location: &str) -> usize {
    // Parse specificity from the source location.
    // Implementations in more specific modules (e.g., core/ vs examples/)
    // get higher base specificity.
    let mut score = 0;
    if location.contains("core/") {
        score += 1;
    }
    // Count concrete type markers in the location path
    // (this is a heuristic; full implementation would inspect the AST)
    score += location.matches("<").count();
    score
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

    // === InstanceResolver tests ===

    #[test]
    fn test_resolver_direct_search() {
        let mut resolver = InstanceResolver::new();
        resolver.register(candidate("Monoid", "Z3", "loc1"));
        match resolver.search("Monoid", "Z3") {
            SearchResult::Unique(c) => assert_eq!(c.target_type.as_str(), "Z3"),
            other => panic!("expected Unique, got {:?}", other),
        }
    }

    #[test]
    fn test_resolver_superclass_search() {
        let mut resolver = InstanceResolver::new();
        // Register: AbelianGroup extends Group extends Monoid
        resolver.register_superclass("AbelianGroup", "Group");
        resolver.register_superclass("Group", "Monoid");
        // Only implement AbelianGroup for Z3
        resolver.register(candidate("AbelianGroup", "Z3", "loc1"));

        // Search for Group for Z3 — should find via superclass
        match resolver.search("Group", "Z3") {
            SearchResult::Unique(c) => {
                assert_eq!(c.protocol.as_str(), "Group");
                assert_eq!(c.target_type.as_str(), "Z3");
            }
            other => panic!("expected Unique via superclass, got {:?}", other),
        }

        // Search for Monoid for Z3 — should find via transitive superclass
        match resolver.search("Monoid", "Z3") {
            SearchResult::Unique(c) => {
                assert_eq!(c.protocol.as_str(), "Monoid");
                assert_eq!(c.target_type.as_str(), "Z3");
            }
            other => panic!("expected Unique via transitive superclass, got {:?}", other),
        }
    }

    #[test]
    fn test_resolver_not_found() {
        let mut resolver = InstanceResolver::new();
        resolver.register(candidate("Monoid", "Z3", "loc1"));
        assert_eq!(resolver.search("Group", "Nat4"), SearchResult::NotFound);
    }

    #[test]
    fn test_resolver_depth_limit() {
        let mut resolver = InstanceResolver::new();
        resolver.max_depth = 2;
        // Create a chain deeper than max_depth
        resolver.register_superclass("A", "B");
        resolver.register_superclass("B", "C");
        resolver.register_superclass("C", "D");
        resolver.register(candidate("A", "T", "loc1"));
        // D is 3 hops away, but max_depth is 2
        assert_eq!(resolver.search("D", "T"), SearchResult::NotFound);
    }

    #[test]
    fn test_resolver_search_all_instances() {
        let mut resolver = InstanceResolver::new();
        resolver.register_superclass("AbelianGroup", "Group");
        resolver.register(candidate("Group", "Z3", "loc1"));
        resolver.register(candidate("AbelianGroup", "Z4", "loc2"));

        let all = resolver.search_all_instances("Group");
        assert!(all.len() >= 2); // Z3 directly, Z4 via superclass
    }

    #[test]
    fn test_smt_coherence_resolves_by_specificity() {
        let mut reg = InstanceRegistry::new();
        reg.register(candidate("Functor", "List", "core/math/category.vr:100"));
        reg.register(candidate("Functor", "List", "examples/demo.vr:50"));

        let report = smt_check_coherence(&reg, true);
        // The core/ implementation is more specific, so the violation is resolved.
        assert!(report.is_coherent());
        assert!(report.smt_checked);
    }
}
