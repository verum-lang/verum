//! Lock Ordering Verification for Deadlock Prevention
//!
//! This module implements compile-time verification of lock acquisition ordering
//! to prevent deadlocks.
//!
//! # Design
//!
//! Lock ordering verification works by:
//! 1. Collecting `@lock_level(level: N)` attributes from type definitions
//! 2. Building a lock acquisition graph from code analysis
//! 3. Verifying that all acquisitions follow the strict partial order (lower -> higher)
//! 4. Detecting cycles which indicate potential deadlock scenarios
//!
//! # Algorithm
//!
//! The verification uses a dataflow-based approach:
//! - Track which locks are held at each program point
//! - When a new lock is acquired, verify level(new_lock) > max(level(held_locks))
//! - Report violation if the ordering constraint is violated
//!
//! # Deadlock Prevention Strategies
//!
//! 1. **Lock Ordering (primary)**: All locks must be acquired in ascending level order.
//!    `@lock_level(level: N)` annotates lock types. Acquiring level L while holding
//!    level M where L <= M is a compile error.
//! 2. **Lock Level Inference**: When no explicit levels, compiler infers ordering from
//!    usage patterns across all code paths and detects inconsistencies.
//! 3. **Timeout-Based Acquisition**: For cases where ordering is impractical, use
//!    `lock_timeout(duration)` with retry logic and backoff.
//! 4. **Static Deadlock Detection**: Compiler tracks lock acquisition patterns across
//!    the entire codebase, building a lock-dependency graph and detecting cycles.
//! 5. **Runtime Detection (dev mode)**: `@deadlock_detection(enabled: true, timeout: T)`
//!    enables runtime monitoring that reports potential deadlocks with full traces.

use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;
use verum_common::{List, Maybe, Text};

/// Lock level identifier
pub type LockLevel = u32;

/// Unique identifier for a lock type
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LockTypeId {
    /// Module path where the lock type is defined
    pub module: Text,
    /// Name of the lock type
    pub name: Text,
}

impl LockTypeId {
    /// Create a new lock type identifier
    pub fn new(module: impl Into<Text>, name: impl Into<Text>) -> Self {
        Self {
            module: module.into(),
            name: name.into(),
        }
    }

    /// Get fully qualified name
    pub fn qualified_name(&self) -> Text {
        Text::from(format!("{}::{}", self.module, self.name))
    }
}

/// Information about a lock type with its assigned level
#[derive(Debug, Clone)]
pub struct LockInfo {
    /// The lock type identifier
    pub id: LockTypeId,
    /// The lock level assigned via @lock_level attribute
    pub level: LockLevel,
    /// Source location where the lock type is defined
    pub source_location: Maybe<SourceLocation>,
}

/// Source location for error reporting
#[derive(Debug, Clone)]
pub struct SourceLocation {
    /// File path
    pub file: Text,
    /// Line number
    pub line: u32,
    /// Column number
    pub column: u32,
}

impl SourceLocation {
    /// Create a new source location
    pub fn new(file: impl Into<Text>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// A lock acquisition in the program
#[derive(Debug, Clone)]
pub struct LockAcquisition {
    /// The lock being acquired
    pub lock_id: LockTypeId,
    /// Location of the acquisition
    pub location: SourceLocation,
    /// Function where the acquisition occurs
    pub function: Text,
}

/// Registry of all known lock types and their levels
#[derive(Debug, Default)]
pub struct LockRegistry {
    /// Map from lock type to its info
    locks: HashMap<LockTypeId, LockInfo>,
}

impl LockRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            locks: HashMap::new(),
        }
    }

    /// Register a lock type with its level
    pub fn register(&mut self, id: LockTypeId, level: LockLevel, location: Maybe<SourceLocation>) {
        self.locks.insert(
            id.clone(),
            LockInfo {
                id,
                level,
                source_location: location,
            },
        );
    }

    /// Get lock info by ID
    pub fn get(&self, id: &LockTypeId) -> Maybe<&LockInfo> {
        self.locks.get(id).map_or(Maybe::None, Maybe::Some)
    }

    /// Get lock level by ID
    pub fn get_level(&self, id: &LockTypeId) -> Maybe<LockLevel> {
        self.locks
            .get(id)
            .map_or(Maybe::None, |info| Maybe::Some(info.level))
    }

    /// Get all registered locks
    pub fn all_locks(&self) -> impl Iterator<Item = &LockInfo> {
        self.locks.values()
    }

    /// Number of registered locks
    pub fn len(&self) -> usize {
        self.locks.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.locks.is_empty()
    }
}

/// Errors during lock ordering verification
#[derive(Debug, Clone, Error)]
pub enum LockOrderingError {
    /// Lock acquired while holding a higher-level lock
    #[error(
        "lock ordering violation: acquiring {acquired} (level {acquired_level}) while holding {held} (level {held_level}) at {location}"
    )]
    OrderViolation {
        /// The lock being acquired
        acquired: Text,
        /// Level of acquired lock
        acquired_level: LockLevel,
        /// The lock already held
        held: Text,
        /// Level of held lock
        held_level: LockLevel,
        /// Location of the violation
        location: SourceLocation,
    },

    /// Potential deadlock cycle detected
    #[error("potential deadlock: cycle detected involving locks: {cycle}")]
    CycleDetected {
        /// Description of the cycle
        cycle: Text,
    },

    /// Lock type not registered
    #[error("unknown lock type: {lock_type} at {location}")]
    UnknownLock {
        /// The unknown lock type
        lock_type: Text,
        /// Location where it's used
        location: SourceLocation,
    },

    /// Lock acquired without level annotation
    #[error("lock type {lock_type} missing @lock_level attribute")]
    MissingLockLevel {
        /// The lock type without level
        lock_type: Text,
    },
}

/// Result type for lock ordering operations
pub type LockOrderingResult<T> = Result<T, LockOrderingError>;

/// Lock acquisition state at a program point
#[derive(Debug, Clone, Default)]
pub struct HeldLocks {
    /// Set of currently held locks with their levels
    pub(crate) locks: HashMap<LockTypeId, LockLevel>,
    /// Maximum level of any held lock
    max_level: LockLevel,
}

impl HeldLocks {
    /// Create empty held locks state
    pub fn new() -> Self {
        Self {
            locks: HashMap::new(),
            max_level: 0,
        }
    }

    /// Acquire a lock
    pub fn acquire(&mut self, id: LockTypeId, level: LockLevel) {
        self.locks.insert(id, level);
        if level > self.max_level {
            self.max_level = level;
        }
    }

    /// Release a lock
    pub fn release(&mut self, id: &LockTypeId) {
        self.locks.remove(id);
        // Recalculate max level
        self.max_level = self.locks.values().copied().max().unwrap_or(0);
    }

    /// Check if acquiring a lock at given level would violate ordering
    pub fn would_violate(&self, level: LockLevel) -> Maybe<(&LockTypeId, LockLevel)> {
        // Find any held lock with level >= the one being acquired
        for (id, &held_level) in &self.locks {
            if held_level >= level {
                return Maybe::Some((id, held_level));
            }
        }
        Maybe::None
    }

    /// Get the maximum level of held locks
    pub fn max_held_level(&self) -> LockLevel {
        self.max_level
    }

    /// Check if any locks are held
    pub fn is_empty(&self) -> bool {
        self.locks.is_empty()
    }

    /// Get number of held locks
    pub fn count(&self) -> usize {
        self.locks.len()
    }
}

/// Lock acquisition graph for cycle detection
#[derive(Debug, Default)]
pub struct LockAcquisitionGraph {
    /// Edges: (lock_a, lock_b) means lock_a was held when lock_b was acquired
    edges: HashSet<(LockTypeId, LockTypeId)>,
}

impl LockAcquisitionGraph {
    /// Create a new empty graph
    pub fn new() -> Self {
        Self {
            edges: HashSet::new(),
        }
    }

    /// Add an edge: lock_a was held when lock_b was acquired
    pub fn add_edge(&mut self, held: LockTypeId, acquired: LockTypeId) {
        self.edges.insert((held, acquired));
    }

    /// Detect cycles in the graph using DFS
    pub fn detect_cycles(&self) -> List<List<LockTypeId>> {
        let mut cycles = List::new();
        let mut visited = HashSet::new();
        let mut path = Vec::new();
        let mut on_stack = HashSet::new();

        // Build adjacency list
        let mut adj: HashMap<&LockTypeId, Vec<&LockTypeId>> = HashMap::new();
        for (from, to) in &self.edges {
            adj.entry(from).or_default().push(to);
        }

        // Get all nodes
        let nodes: HashSet<_> = self.edges.iter().flat_map(|(a, b)| vec![a, b]).collect();

        // DFS from each unvisited node
        for start in nodes {
            if !visited.contains(start) {
                self.dfs_cycle_detection(
                    start,
                    &adj,
                    &mut visited,
                    &mut on_stack,
                    &mut path,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    fn dfs_cycle_detection<'a>(
        &'a self,
        node: &'a LockTypeId,
        adj: &HashMap<&'a LockTypeId, Vec<&'a LockTypeId>>,
        visited: &mut HashSet<&'a LockTypeId>,
        on_stack: &mut HashSet<&'a LockTypeId>,
        path: &mut Vec<&'a LockTypeId>,
        cycles: &mut List<List<LockTypeId>>,
    ) {
        visited.insert(node);
        on_stack.insert(node);
        path.push(node);

        if let Some(neighbors) = adj.get(&node) {
            for neighbor in neighbors.iter().copied() {
                if !visited.contains(neighbor) {
                    self.dfs_cycle_detection(neighbor, adj, visited, on_stack, path, cycles);
                } else if on_stack.contains(neighbor) {
                    // Found a cycle! Extract it from path
                    let mut cycle = List::new();
                    let mut in_cycle = false;
                    for n in path.iter().copied() {
                        if std::ptr::eq(n, neighbor) {
                            in_cycle = true;
                        }
                        if in_cycle {
                            cycle.push(n.clone());
                        }
                    }
                    cycle.push(neighbor.clone());
                    cycles.push(cycle);
                }
            }
        }

        path.pop();
        on_stack.remove(node);
    }
}

/// Lock ordering verifier
#[derive(Debug)]
pub struct LockOrderingVerifier {
    /// Registry of known lock types
    registry: LockRegistry,
    /// Acquisition graph for cycle detection
    graph: LockAcquisitionGraph,
    /// Collected violations
    violations: List<LockOrderingError>,
    /// Configuration
    config: LockOrderingConfig,
}

/// Configuration for lock ordering verification
#[derive(Debug, Clone)]
pub struct LockOrderingConfig {
    /// Whether to require @lock_level on all mutex types
    pub require_levels: bool,
    /// Whether to detect cycles (more expensive)
    pub detect_cycles: bool,
    /// Whether to report unknown locks as errors
    pub strict_mode: bool,
}

impl Default for LockOrderingConfig {
    fn default() -> Self {
        Self {
            require_levels: true,
            detect_cycles: true,
            strict_mode: false,
        }
    }
}

impl LockOrderingVerifier {
    /// Create a new verifier with default configuration
    pub fn new() -> Self {
        Self {
            registry: LockRegistry::new(),
            graph: LockAcquisitionGraph::new(),
            violations: List::new(),
            config: LockOrderingConfig::default(),
        }
    }

    /// Create a new verifier with custom configuration
    pub fn with_config(config: LockOrderingConfig) -> Self {
        Self {
            registry: LockRegistry::new(),
            graph: LockAcquisitionGraph::new(),
            violations: List::new(),
            config,
        }
    }

    /// Register a lock type from @lock_level attribute
    pub fn register_lock(
        &mut self,
        module: impl Into<Text>,
        name: impl Into<Text>,
        level: LockLevel,
        location: Maybe<SourceLocation>,
    ) {
        let id = LockTypeId::new(module, name);
        self.registry.register(id, level, location);
    }

    /// Verify a lock acquisition
    pub fn verify_acquisition(
        &mut self,
        held: &HeldLocks,
        lock_id: &LockTypeId,
        location: SourceLocation,
    ) -> LockOrderingResult<()> {
        // Check if lock type is known.
        //
        // Three policy gates compose here:
        //
        //   * `require_levels` (declaration-layer): the lock TYPE must
        //     have a `@lock_level` annotation registered.  A type
        //     without a level can't be ordered against anything else,
        //     so under `require_levels = true` we surface the
        //     declaration gap as `MissingLockLevel` — a directed error
        //     that points the user at the unannotated type definition.
        //
        //   * `strict_mode` (use-layer): even when we don't require
        //     declaration coverage globally, the use-site for an
        //     unknown lock is still suspect — strict mode promotes the
        //     skip-and-pass to `UnknownLock`.
        //
        //   * Default (lenient): unknown lock + neither flag set →
        //     `Ok(())`, skip verification.  The caller has decided
        //     the analysis is best-effort and missing annotations
        //     don't gate compilation.
        //
        // Pre-fix `require_levels` was an inert config field — the
        // verifier never read it.  This wiring closes the same
        // architectural anti-pattern documented for the bytecode
        // validator + content/dependency hash + validate_on_extract:
        // a public field claims a security/safety contract that no
        // code path enforced.
        let lock_level = match self.registry.get_level(lock_id) {
            Maybe::Some(level) => level,
            Maybe::None => {
                if self.config.require_levels {
                    return Err(LockOrderingError::MissingLockLevel {
                        lock_type: lock_id.qualified_name(),
                    });
                }
                if self.config.strict_mode {
                    return Err(LockOrderingError::UnknownLock {
                        lock_type: lock_id.qualified_name(),
                        location,
                    });
                }
                // Unknown lock - skip verification
                return Ok(());
            }
        };

        // Check ordering violation
        if let Maybe::Some((held_id, held_level)) = held.would_violate(lock_level) {
            let err = LockOrderingError::OrderViolation {
                acquired: lock_id.qualified_name(),
                acquired_level: lock_level,
                held: held_id.clone().qualified_name(),
                held_level,
                location,
            };
            self.violations.push(err.clone());
            return Err(err);
        }

        // Add edges to graph for cycle detection
        if self.config.detect_cycles {
            for held_id in held.locks.keys() {
                self.graph.add_edge(held_id.clone(), lock_id.clone());
            }
        }

        Ok(())
    }

    /// Run cycle detection on the acquisition graph
    pub fn detect_deadlock_cycles(&self) -> List<List<LockTypeId>> {
        self.graph.detect_cycles()
    }

    /// Get all collected violations
    pub fn violations(&self) -> &List<LockOrderingError> {
        &self.violations
    }

    /// Clear violations
    pub fn clear_violations(&mut self) {
        self.violations.clear();
    }

    /// Get the lock registry
    pub fn registry(&self) -> &LockRegistry {
        &self.registry
    }

    /// Check if verification passed with no violations
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }
}

impl Default for LockOrderingVerifier {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about lock ordering verification
#[derive(Debug, Default)]
pub struct LockOrderingStats {
    /// Number of lock types registered
    pub lock_types: usize,
    /// Number of acquisitions verified
    pub acquisitions_verified: usize,
    /// Number of ordering violations found
    pub violations: usize,
    /// Number of potential deadlock cycles
    pub deadlock_cycles: usize,
}

impl LockOrderingStats {
    /// Create new stats
    pub fn new() -> Self {
        Self::default()
    }
}

/// Convenience function to verify a single acquisition
pub fn verify_lock_acquisition(
    registry: &LockRegistry,
    held: &HeldLocks,
    lock_id: &LockTypeId,
) -> LockOrderingResult<()> {
    let lock_level = match registry.get_level(lock_id) {
        Maybe::Some(level) => level,
        Maybe::None => return Ok(()), // Unknown locks not verified
    };

    if let Maybe::Some((held_id, held_level)) = held.would_violate(lock_level) {
        return Err(LockOrderingError::OrderViolation {
            acquired: lock_id.qualified_name(),
            acquired_level: lock_level,
            held: held_id.clone().qualified_name(),
            held_level,
            location: SourceLocation::new("<unknown>", 0, 0),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_ordering_basic() {
        let mut registry = LockRegistry::new();
        registry.register(LockTypeId::new("db", "DatabaseLock"), 1, Maybe::None);
        registry.register(LockTypeId::new("cache", "CacheLock"), 2, Maybe::None);

        let mut held = HeldLocks::new();

        // Acquire database lock (level 1) - should succeed
        let db_id = LockTypeId::new("db", "DatabaseLock");
        assert!(verify_lock_acquisition(&registry, &held, &db_id).is_ok());
        held.acquire(db_id.clone(), 1);

        // Acquire cache lock (level 2) while holding database - should succeed
        let cache_id = LockTypeId::new("cache", "CacheLock");
        assert!(verify_lock_acquisition(&registry, &held, &cache_id).is_ok());
    }

    #[test]
    fn test_lock_ordering_violation() {
        let mut registry = LockRegistry::new();
        registry.register(LockTypeId::new("db", "DatabaseLock"), 1, Maybe::None);
        registry.register(LockTypeId::new("cache", "CacheLock"), 2, Maybe::None);

        let mut held = HeldLocks::new();

        // Acquire cache lock (level 2) first
        let cache_id = LockTypeId::new("cache", "CacheLock");
        held.acquire(cache_id.clone(), 2);

        // Try to acquire database lock (level 1) - should fail
        let db_id = LockTypeId::new("db", "DatabaseLock");
        let result = verify_lock_acquisition(&registry, &held, &db_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = LockAcquisitionGraph::new();

        let a = LockTypeId::new("", "A");
        let b = LockTypeId::new("", "B");
        let c = LockTypeId::new("", "C");

        // Create cycle: A -> B -> C -> A
        graph.add_edge(a.clone(), b.clone());
        graph.add_edge(b.clone(), c.clone());
        graph.add_edge(c.clone(), a.clone());

        let cycles = graph.detect_cycles();
        assert!(!cycles.is_empty(), "Should detect cycle");
    }

    // -------------------------------------------------------------------------
    // require_levels gate — pin the three-gate policy on unknown locks.
    //
    // Pre-fix the field was inert; this test asserts each gate's distinct
    // outcome on the same input (an unregistered lock type at use-site).
    // -------------------------------------------------------------------------

    fn make_verifier_with(
        require_levels: bool,
        strict_mode: bool,
    ) -> LockOrderingVerifier {
        let cfg = LockOrderingConfig {
            require_levels,
            detect_cycles: false,
            strict_mode,
        };
        LockOrderingVerifier::with_config(cfg)
    }

    fn dummy_loc() -> SourceLocation {
        SourceLocation {
            file: Text::from(""),
            line: 0,
            column: 0,
        }
    }

    #[test]
    fn require_levels_true_rejects_unregistered_lock_with_missing_lock_level() {
        let mut v = make_verifier_with(/*require_levels=*/ true, /*strict=*/ false);
        let unknown = LockTypeId::new("user", "UndeclaredMutex");
        let held = HeldLocks::new();
        match v.verify_acquisition(&held, &unknown, dummy_loc()) {
            Err(LockOrderingError::MissingLockLevel { lock_type }) => {
                assert!(
                    lock_type.as_str().contains("UndeclaredMutex"),
                    "MissingLockLevel must carry the qualified type name; got {:?}",
                    lock_type,
                );
            }
            other => panic!(
                "expected MissingLockLevel under require_levels=true, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn strict_mode_true_with_levels_off_rejects_unknown_lock() {
        // require_levels=false, strict_mode=true: use-site unknown
        // surfaces as UnknownLock (not MissingLockLevel — the
        // declaration-layer gate is off).
        let mut v = make_verifier_with(/*require_levels=*/ false, /*strict=*/ true);
        let unknown = LockTypeId::new("user", "UndeclaredMutex");
        let held = HeldLocks::new();
        match v.verify_acquisition(&held, &unknown, dummy_loc()) {
            Err(LockOrderingError::UnknownLock { .. }) => {}
            other => panic!(
                "expected UnknownLock under strict_mode=true (require_levels=false), got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn lenient_mode_skips_unknown_lock() {
        // Both gates off — the analysis runs in best-effort mode and
        // an unknown lock simply skips verification.
        let mut v = make_verifier_with(/*require_levels=*/ false, /*strict=*/ false);
        let unknown = LockTypeId::new("user", "UndeclaredMutex");
        let held = HeldLocks::new();
        v.verify_acquisition(&held, &unknown, dummy_loc())
            .expect("lenient mode must skip-and-pass on unknown lock");
    }

    #[test]
    fn require_levels_takes_precedence_over_strict_mode() {
        // Both gates on — require_levels (the declaration-layer
        // diagnostic) wins because MissingLockLevel points at the
        // type definition, which is more actionable than UnknownLock
        // pointing at a use site.
        let mut v = make_verifier_with(/*require_levels=*/ true, /*strict=*/ true);
        let unknown = LockTypeId::new("user", "UndeclaredMutex");
        let held = HeldLocks::new();
        match v.verify_acquisition(&held, &unknown, dummy_loc()) {
            Err(LockOrderingError::MissingLockLevel { .. }) => {}
            other => panic!(
                "expected MissingLockLevel to take precedence, got: {:?}",
                other
            ),
        }
    }
}
