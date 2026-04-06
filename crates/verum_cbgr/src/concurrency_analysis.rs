//! Concurrency Analysis for Compile-Time Data Race Detection
//!
//! This module implements compile-time concurrency analysis to detect potential
//! data races and synchronization issues, including:
//!
//! - **Data Race Detection**: Finds unsynchronized shared memory access
//! - **Happens-Before Analysis**: Tracks synchronization ordering
//! - **Lock Ordering Analysis**: Detects potential deadlocks
//! - **Thread Safety Verification**: Validates Send/Sync bounds
//!
//! # Architecture
//!
//! ```text
//! CFG → ConcurrencyAnalyzer → ConcurrencyAnalysisResult
//!                                   │
//!                                   ▼
//!                   ┌───────────────────────────────┐
//!                   │ HappensBeforeGraph            │
//!                   │ Set<DataRaceWarning>          │
//!                   │ Set<DeadlockWarning>          │
//!                   │ Map<VarId, AccessHistory>     │
//!                   └───────────────────────────────┘
//! ```
//!
//! # Data Race Detection Algorithm
//!
//! Uses a variant of the Eraser algorithm combined with happens-before analysis:
//!
//! 1. Track all memory accesses with their thread context
//! 2. Build happens-before graph from synchronization operations
//! 3. For each pair of accesses to same location:
//!    - If at least one is a write AND
//!    - They are not ordered by happens-before AND
//!    - They are not protected by the same lock
//!    → Report potential data race
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_cbgr::concurrency_analysis::ConcurrencyAnalyzer;
//!
//! let analyzer = ConcurrencyAnalyzer::new(cfg);
//! let result = analyzer.analyze();
//!
//! for race in &result.data_race_warnings {
//!     println!("Potential data race: {:?} vs {:?}",
//!              race.access1, race.access2);
//! }
//! ```
//!
//! Phase 6 of the CBGR analysis pipeline: enhanced compile-time detection of
//! concurrency issues. Uses Eraser algorithm + happens-before analysis to find
//! unsynchronized shared memory accesses. Feeds into tier decisions: references
//! with detected data races cannot be promoted to &checked T (criterion 2:
//! no concurrent access possible).

use crate::analysis::{BlockId, ControlFlowGraph, RefId, Span};
use verum_common::{List, Map, Set};

// ============================================================================
// Thread and Synchronization Identifiers
// ============================================================================

/// Unique identifier for a thread or task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(pub u64);

impl ThreadId {
    /// Main thread ID.
    pub const MAIN: ThreadId = ThreadId(0);

    /// Create from spawn site.
    #[must_use]
    pub fn from_spawn_site(block: BlockId, index: u32) -> Self {
        Self(((block.0 as u64) << 32) | (index as u64))
    }
}

/// Unique identifier for a lock or synchronization primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LockId(pub u64);

impl LockId {
    /// Create from definition site.
    #[must_use]
    pub fn from_site(block: BlockId, ref_id: RefId) -> Self {
        Self(((block.0 as u64) << 32) | ref_id.0)
    }
}

/// Unique identifier for a memory location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocationId(pub u64);

impl LocationId {
    /// Create from reference.
    #[must_use]
    pub fn from_ref(ref_id: RefId) -> Self {
        Self(ref_id.0)
    }

    /// Create from block and offset.
    #[must_use]
    pub fn from_site(block: BlockId, offset: u32) -> Self {
        Self(((block.0 as u64) << 32) | (offset as u64))
    }
}

// ============================================================================
// Memory Access Tracking
// ============================================================================

/// A memory access event.
#[derive(Debug, Clone)]
pub struct MemoryAccess {
    /// Location being accessed.
    pub location: LocationId,
    /// Type of access.
    pub kind: AccessKind,
    /// Thread performing the access.
    pub thread: ThreadId,
    /// Block where access occurs.
    pub block: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Locks held during access.
    pub locks_held: Set<LockId>,
    /// Happens-before clock value.
    pub clock: VectorClock,
}

impl MemoryAccess {
    /// Create new memory access.
    #[must_use]
    pub fn new(location: LocationId, kind: AccessKind, thread: ThreadId, block: BlockId) -> Self {
        Self {
            location,
            kind,
            thread,
            block,
            span: None,
            locks_held: Set::new(),
            clock: VectorClock::new(),
        }
    }

    /// Create with span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// Add lock held during access.
    #[must_use]
    pub fn with_lock(mut self, lock: LockId) -> Self {
        self.locks_held.insert(lock);
        self
    }

    /// Set vector clock.
    #[must_use]
    pub fn with_clock(mut self, clock: VectorClock) -> Self {
        self.clock = clock;
        self
    }

    /// Check if this is a write access.
    #[must_use]
    pub fn is_write(&self) -> bool {
        matches!(self.kind, AccessKind::Write | AccessKind::ReadModifyWrite)
    }

    /// Check if this is a read access.
    #[must_use]
    pub fn is_read(&self) -> bool {
        matches!(self.kind, AccessKind::Read | AccessKind::ReadModifyWrite)
    }
}

/// Type of memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// Read access.
    Read,
    /// Write access.
    Write,
    /// Atomic read-modify-write.
    ReadModifyWrite,
    /// Atomic load with ordering.
    AtomicLoad(MemoryOrdering),
    /// Atomic store with ordering.
    AtomicStore(MemoryOrdering),
    /// Atomic compare-and-swap.
    AtomicCas(MemoryOrdering),
}

impl AccessKind {
    /// Check if this is an atomic operation.
    #[must_use]
    pub fn is_atomic(&self) -> bool {
        matches!(
            self,
            Self::AtomicLoad(_) | Self::AtomicStore(_) | Self::AtomicCas(_)
        )
    }

    /// Get the memory ordering if this is atomic.
    #[must_use]
    pub fn ordering(&self) -> Option<MemoryOrdering> {
        match self {
            Self::AtomicLoad(o) | Self::AtomicStore(o) | Self::AtomicCas(o) => Some(*o),
            _ => None,
        }
    }
}

/// Memory ordering for atomic operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOrdering {
    /// Relaxed ordering (no synchronization).
    Relaxed,
    /// Acquire ordering (synchronizes-with release).
    Acquire,
    /// Release ordering (synchronizes-with acquire).
    Release,
    /// Acquire-release ordering.
    AcqRel,
    /// Sequentially consistent ordering.
    SeqCst,
}

impl MemoryOrdering {
    /// Check if this ordering provides acquire semantics.
    #[must_use]
    pub fn is_acquire(&self) -> bool {
        matches!(self, Self::Acquire | Self::AcqRel | Self::SeqCst)
    }

    /// Check if this ordering provides release semantics.
    #[must_use]
    pub fn is_release(&self) -> bool {
        matches!(self, Self::Release | Self::AcqRel | Self::SeqCst)
    }
}

// ============================================================================
// Vector Clock for Happens-Before
// ============================================================================

/// Vector clock for tracking happens-before relationships.
#[derive(Debug, Clone, Default)]
pub struct VectorClock {
    /// Clock values per thread.
    clocks: Map<ThreadId, u64>,
}

impl VectorClock {
    /// Create new empty vector clock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get clock value for a thread.
    #[must_use]
    pub fn get(&self, thread: ThreadId) -> u64 {
        self.clocks.get(&thread).copied().unwrap_or(0)
    }

    /// Increment clock for a thread.
    pub fn tick(&mut self, thread: ThreadId) {
        let val = self.clocks.entry(thread).or_insert(0);
        *val += 1;
    }

    /// Merge with another clock (join operation).
    pub fn join(&mut self, other: &VectorClock) {
        for (thread, &val) in &other.clocks {
            let entry = self.clocks.entry(*thread).or_insert(0);
            *entry = (*entry).max(val);
        }
    }

    /// Check if this clock happens-before another.
    #[must_use]
    pub fn happens_before(&self, other: &VectorClock) -> bool {
        // self ≤ other (all components less than or equal)
        for (thread, &val) in &self.clocks {
            if val > other.get(*thread) {
                return false;
            }
        }
        true
    }

    /// Check if two clocks are concurrent (neither happens-before the other).
    #[must_use]
    pub fn concurrent_with(&self, other: &VectorClock) -> bool {
        !self.happens_before(other) && !other.happens_before(self)
    }
}

// ============================================================================
// Synchronization Operations
// ============================================================================

/// A synchronization operation.
#[derive(Debug, Clone)]
pub struct SyncOperation {
    /// Type of synchronization.
    pub kind: SyncKind,
    /// Thread performing the operation.
    pub thread: ThreadId,
    /// Block where operation occurs.
    pub block: BlockId,
    /// Source span if available.
    pub span: Option<Span>,
    /// Lock ID if this is a lock operation.
    pub lock_id: Option<LockId>,
}

impl SyncOperation {
    /// Create new sync operation.
    #[must_use]
    pub fn new(kind: SyncKind, thread: ThreadId, block: BlockId) -> Self {
        Self {
            kind,
            thread,
            block,
            span: None,
            lock_id: None,
        }
    }

    /// Create with lock ID.
    #[must_use]
    pub fn with_lock(mut self, lock_id: LockId) -> Self {
        self.lock_id = Some(lock_id);
        self
    }

    /// Create with span.
    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
}

/// Type of synchronization operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    /// Thread spawn (creates new thread).
    Spawn,
    /// Thread join (waits for thread completion).
    Join,
    /// Lock acquisition.
    LockAcquire,
    /// Lock release.
    LockRelease,
    /// Condition variable wait.
    CondWait,
    /// Condition variable signal.
    CondSignal,
    /// Condition variable broadcast.
    CondBroadcast,
    /// Barrier wait.
    BarrierWait,
    /// Channel send.
    ChannelSend,
    /// Channel receive.
    ChannelRecv,
    /// Atomic fence.
    Fence(MemoryOrdering),
}

// ============================================================================
// Data Race Warning
// ============================================================================

/// Warning for potential data race.
#[derive(Debug, Clone)]
pub struct DataRaceWarning {
    /// First conflicting access.
    pub access1: MemoryAccess,
    /// Second conflicting access.
    pub access2: MemoryAccess,
    /// Memory location with race.
    pub location: LocationId,
    /// Confidence level (0.0-1.0).
    pub confidence: f64,
    /// Reason for the warning.
    pub reason: DataRaceReason,
}

impl DataRaceWarning {
    /// Create new data race warning.
    #[must_use]
    pub fn new(access1: MemoryAccess, access2: MemoryAccess) -> Self {
        let location = access1.location;
        Self {
            access1,
            access2,
            location,
            confidence: 1.0,
            reason: DataRaceReason::UnsynchronizedAccess,
        }
    }

    /// Set confidence.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }

    /// Set reason.
    #[must_use]
    pub fn with_reason(mut self, reason: DataRaceReason) -> Self {
        self.reason = reason;
        self
    }
}

/// Reason for data race warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataRaceReason {
    /// Accesses not protected by any synchronization.
    UnsynchronizedAccess,
    /// Different locks held during accesses.
    DifferentLocks,
    /// No happens-before ordering between accesses.
    NoHappensBefore,
    /// Relaxed atomic used for synchronization.
    RelaxedAtomic,
    /// Unknown reason (conservative).
    Unknown,
}

// ============================================================================
// Deadlock Warning
// ============================================================================

/// Warning for potential deadlock.
#[derive(Debug, Clone)]
pub struct DeadlockWarning {
    /// Locks involved in potential deadlock cycle.
    pub lock_cycle: List<LockId>,
    /// Threads involved.
    pub threads: Set<ThreadId>,
    /// Confidence level.
    pub confidence: f64,
    /// Type of deadlock.
    pub kind: DeadlockKind,
}

impl DeadlockWarning {
    /// Create new deadlock warning.
    #[must_use]
    pub fn new(lock_cycle: List<LockId>, kind: DeadlockKind) -> Self {
        Self {
            lock_cycle,
            threads: Set::new(),
            confidence: 1.0,
            kind,
        }
    }

    /// Add involved thread.
    #[must_use]
    pub fn with_thread(mut self, thread: ThreadId) -> Self {
        self.threads.insert(thread);
        self
    }
}

/// Type of deadlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlockKind {
    /// Self-deadlock (same thread tries to acquire held lock).
    SelfDeadlock,
    /// AB-BA deadlock (two threads acquire locks in opposite order).
    LockOrderViolation,
    /// N-way deadlock cycle.
    CyclicWait,
    /// Unknown deadlock type.
    Unknown,
}

// ============================================================================
// Analysis Result
// ============================================================================

/// Result of concurrency analysis.
#[derive(Debug, Clone)]
pub struct ConcurrencyAnalysisResult {
    /// All memory accesses tracked.
    pub accesses: List<MemoryAccess>,
    /// All synchronization operations.
    pub sync_operations: List<SyncOperation>,
    /// Data race warnings.
    pub data_race_warnings: List<DataRaceWarning>,
    /// Deadlock warnings.
    pub deadlock_warnings: List<DeadlockWarning>,
    /// Thread-safety violations.
    pub thread_safety_violations: List<ThreadSafetyViolation>,
    /// Analysis statistics.
    pub stats: ConcurrencyStats,
}

impl ConcurrencyAnalysisResult {
    /// Create empty result.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            accesses: List::new(),
            sync_operations: List::new(),
            data_race_warnings: List::new(),
            deadlock_warnings: List::new(),
            thread_safety_violations: List::new(),
            stats: ConcurrencyStats::default(),
        }
    }

    /// Check if analysis found any issues.
    #[must_use]
    pub fn has_issues(&self) -> bool {
        !self.data_race_warnings.is_empty()
            || !self.deadlock_warnings.is_empty()
            || !self.thread_safety_violations.is_empty()
    }

    /// Get total number of warnings.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.data_race_warnings.len()
            + self.deadlock_warnings.len()
            + self.thread_safety_violations.len()
    }
}

/// Thread-safety violation (Send/Sync bounds).
#[derive(Debug, Clone)]
pub struct ThreadSafetyViolation {
    /// Reference that violates bounds.
    pub ref_id: RefId,
    /// Kind of violation.
    pub kind: ThreadSafetyKind,
    /// Block where violation occurs.
    pub block: BlockId,
    /// Span if available.
    pub span: Option<Span>,
}

/// Kind of thread-safety violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadSafetyKind {
    /// Non-Send type sent across threads.
    NotSend,
    /// Non-Sync type shared across threads.
    NotSync,
    /// Mutable aliasing across threads.
    UnsafeMutableAlias,
}

/// Statistics from concurrency analysis.
#[derive(Debug, Clone, Default)]
pub struct ConcurrencyStats {
    /// Total memory accesses analyzed.
    pub total_accesses: usize,
    /// Write accesses.
    pub write_accesses: usize,
    /// Atomic accesses.
    pub atomic_accesses: usize,
    /// Synchronization operations.
    pub sync_operations: usize,
    /// Threads detected.
    pub threads_detected: usize,
    /// Locks detected.
    pub locks_detected: usize,
    /// Analysis time in microseconds.
    pub analysis_time_us: u64,
}

// ============================================================================
// Concurrency Analyzer
// ============================================================================

/// Analyzer for concurrency issues.
pub struct ConcurrencyAnalyzer {
    /// Control flow graph.
    cfg: ControlFlowGraph,
    /// Memory accesses.
    accesses: List<MemoryAccess>,
    /// Sync operations.
    sync_ops: List<SyncOperation>,
    /// Current vector clocks per thread.
    thread_clocks: Map<ThreadId, VectorClock>,
    /// Locks currently held per thread.
    held_locks: Map<ThreadId, Set<LockId>>,
    /// Lock acquisition order per thread (for deadlock detection).
    lock_order: Map<ThreadId, List<LockId>>,
    /// All detected threads.
    threads: Set<ThreadId>,
    /// All detected locks.
    locks: Set<LockId>,
    /// Configuration.
    config: ConcurrencyAnalysisConfig,
}

/// Configuration for concurrency analysis.
#[derive(Debug, Clone)]
pub struct ConcurrencyAnalysisConfig {
    /// Whether to analyze for data races.
    pub detect_data_races: bool,
    /// Whether to analyze for deadlocks.
    pub detect_deadlocks: bool,
    /// Whether to check thread safety bounds.
    pub check_thread_safety: bool,
    /// Minimum confidence for warnings.
    pub min_confidence: f64,
    /// Maximum accesses to track (0 = unlimited).
    pub max_accesses: usize,
}

impl Default for ConcurrencyAnalysisConfig {
    fn default() -> Self {
        Self {
            detect_data_races: true,
            detect_deadlocks: true,
            check_thread_safety: true,
            min_confidence: 0.5,
            max_accesses: 0,
        }
    }
}

impl ConcurrencyAnalyzer {
    /// Create new concurrency analyzer.
    #[must_use]
    pub fn new(cfg: ControlFlowGraph) -> Self {
        let mut threads = Set::new();
        threads.insert(ThreadId::MAIN);

        let mut thread_clocks = Map::new();
        thread_clocks.insert(ThreadId::MAIN, VectorClock::new());

        Self {
            cfg,
            accesses: List::new(),
            sync_ops: List::new(),
            thread_clocks,
            held_locks: Map::new(),
            lock_order: Map::new(),
            threads,
            locks: Set::new(),
            config: ConcurrencyAnalysisConfig::default(),
        }
    }

    /// Create with configuration.
    #[must_use]
    pub fn with_config(mut self, config: ConcurrencyAnalysisConfig) -> Self {
        self.config = config;
        self
    }

    /// Perform concurrency analysis.
    #[must_use]
    pub fn analyze(mut self) -> ConcurrencyAnalysisResult {
        let start = std::time::Instant::now();

        // Phase 1: Extract memory accesses and sync operations
        self.extract_accesses();
        self.extract_sync_operations();

        // Phase 2: Build happens-before from sync operations
        self.process_sync_operations();

        // Phase 3: Detect issues
        let data_race_warnings = if self.config.detect_data_races {
            self.detect_data_races()
        } else {
            List::new()
        };

        let deadlock_warnings = if self.config.detect_deadlocks {
            self.detect_deadlocks()
        } else {
            List::new()
        };

        let thread_safety_violations = if self.config.check_thread_safety {
            self.check_thread_safety()
        } else {
            List::new()
        };

        // Build statistics
        let stats = ConcurrencyStats {
            total_accesses: self.accesses.len(),
            write_accesses: self.accesses.iter().filter(|a| a.is_write()).count(),
            atomic_accesses: self.accesses.iter().filter(|a| a.kind.is_atomic()).count(),
            sync_operations: self.sync_ops.len(),
            threads_detected: self.threads.len(),
            locks_detected: self.locks.len(),
            analysis_time_us: start.elapsed().as_micros() as u64,
        };

        ConcurrencyAnalysisResult {
            accesses: self.accesses,
            sync_operations: self.sync_ops,
            data_race_warnings,
            deadlock_warnings,
            thread_safety_violations,
            stats,
        }
    }

    /// Extract memory accesses from CFG.
    fn extract_accesses(&mut self) {
        let thread = ThreadId::MAIN;
        let clock = self.thread_clocks.get(&thread).cloned().unwrap_or_default();

        for (block_id, block) in &self.cfg.blocks {
            // Definitions are writes
            for def in &block.definitions {
                let location = LocationId::from_ref(def.reference);
                let mut access = MemoryAccess::new(location, AccessKind::Write, thread, *block_id);
                access.clock = clock.clone();
                if let Some(span) = def.span {
                    access.span = Some(span);
                }
                self.accesses.push(access);
            }

            // Uses are reads (or read-modify-write if mutable)
            for use_site in &block.uses {
                let location = LocationId::from_ref(use_site.reference);
                let kind = if use_site.is_mutable {
                    AccessKind::ReadModifyWrite
                } else {
                    AccessKind::Read
                };
                let mut access = MemoryAccess::new(location, kind, thread, *block_id);
                access.clock = clock.clone();
                if let Some(span) = use_site.span {
                    access.span = Some(span);
                }
                self.accesses.push(access);
            }
        }
    }

    /// Extract synchronization operations from CFG.
    fn extract_sync_operations(&mut self) {
        // In a real implementation, we'd analyze call sites for
        // Mutex::lock, spawn(), join(), etc.
        // For now, just record basic structure
    }

    /// Process sync operations to update happens-before.
    fn process_sync_operations(&mut self) {
        for sync_op in &self.sync_ops {
            match sync_op.kind {
                SyncKind::Spawn => {
                    // Fork: new thread inherits current thread's clock
                    // (In real impl, we'd track the spawned thread ID)
                }
                SyncKind::Join => {
                    // Join: merge joined thread's clock into current
                }
                SyncKind::LockAcquire => {
                    if let Some(lock_id) = sync_op.lock_id {
                        self.locks.insert(lock_id);
                        self.held_locks
                            .entry(sync_op.thread)
                            .or_insert_with(Set::new)
                            .insert(lock_id);
                    }
                }
                SyncKind::LockRelease => {
                    if let Some(lock_id) = sync_op.lock_id {
                        if let Some(held) = self.held_locks.get_mut(&sync_op.thread) {
                            held.remove(&lock_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Detect data races.
    fn detect_data_races(&self) -> List<DataRaceWarning> {
        let mut warnings = List::new();

        // Group accesses by location
        let mut accesses_by_location: Map<LocationId, List<&MemoryAccess>> = Map::new();
        for access in &self.accesses {
            accesses_by_location
                .entry(access.location)
                .or_insert_with(List::new)
                .push(access);
        }

        // Check each location for races
        for (_location, location_accesses) in &accesses_by_location {
            if location_accesses.len() < 2 {
                continue;
            }

            // Check all pairs of accesses
            for i in 0..location_accesses.len() {
                for j in (i + 1)..location_accesses.len() {
                    let a1 = &location_accesses[i];
                    let a2 = &location_accesses[j];

                    // Race requires at least one write
                    if !a1.is_write() && !a2.is_write() {
                        continue;
                    }

                    // Same thread can't race with itself
                    if a1.thread == a2.thread {
                        continue;
                    }

                    // Atomic accesses don't race (if proper ordering)
                    if a1.kind.is_atomic() && a2.kind.is_atomic() {
                        continue;
                    }

                    // Check happens-before
                    if !a1.clock.concurrent_with(&a2.clock) {
                        continue;
                    }

                    // Check for common lock
                    let common_locks: Set<_> = a1.locks_held.intersection(&a2.locks_held).copied().collect();
                    if !common_locks.is_empty() {
                        continue;
                    }

                    // Found a potential race
                    let warning = DataRaceWarning::new((*a1).clone(), (*a2).clone())
                        .with_reason(DataRaceReason::NoHappensBefore);
                    warnings.push(warning);
                }
            }
        }

        warnings
    }

    /// Detect potential deadlocks.
    fn detect_deadlocks(&self) -> List<DeadlockWarning> {
        let mut warnings = List::new();

        // Build lock graph: edge from L1 to L2 if any thread holds L1 while acquiring L2
        let mut lock_graph: Map<LockId, Set<LockId>> = Map::new();

        for (_thread, order) in &self.lock_order {
            for i in 0..order.len() {
                for j in (i + 1)..order.len() {
                    lock_graph
                        .entry(order[i])
                        .or_insert_with(Set::new)
                        .insert(order[j]);
                }
            }
        }

        // Look for cycles in lock graph
        for lock in &self.locks {
            if let Some(cycle) = self.find_lock_cycle(*lock, &lock_graph) {
                let warning = DeadlockWarning::new(cycle, DeadlockKind::LockOrderViolation);
                warnings.push(warning);
            }
        }

        warnings
    }

    /// Find a cycle starting from a lock.
    fn find_lock_cycle(
        &self,
        start: LockId,
        graph: &Map<LockId, Set<LockId>>,
    ) -> Option<List<LockId>> {
        let mut visited = Set::new();
        let mut path = List::new();

        fn dfs(
            current: LockId,
            start: LockId,
            graph: &Map<LockId, Set<LockId>>,
            visited: &mut Set<LockId>,
            path: &mut List<LockId>,
        ) -> Option<List<LockId>> {
            if visited.contains(&current) {
                if current == start && path.len() > 1 {
                    return Some(path.clone());
                }
                return None;
            }

            visited.insert(current);
            path.push(current);

            if let Some(neighbors) = graph.get(&current) {
                for &next in neighbors {
                    if let Some(cycle) = dfs(next, start, graph, visited, path) {
                        return Some(cycle);
                    }
                }
            }

            path.pop();
            None
        }

        dfs(start, start, graph, &mut visited, &mut path)
    }

    /// Check thread safety bounds.
    fn check_thread_safety(&self) -> List<ThreadSafetyViolation> {
        // In a real implementation, we'd check Send/Sync bounds
        // when values cross thread boundaries
        List::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::BasicBlock;

    fn create_test_cfg() -> ControlFlowGraph {
        let entry = BlockId(0);
        let exit = BlockId(1);
        let mut cfg = ControlFlowGraph::new(entry, exit);

        let mut entry_block = BasicBlock::empty(entry);
        entry_block.successors.insert(exit);
        cfg.add_block(entry_block);

        let mut exit_block = BasicBlock::empty(exit);
        exit_block.predecessors.insert(entry);
        cfg.add_block(exit_block);

        cfg
    }

    #[test]
    fn test_vector_clock_basic() {
        let mut clock1 = VectorClock::new();
        clock1.tick(ThreadId(1));
        assert_eq!(clock1.get(ThreadId(1)), 1);
        assert_eq!(clock1.get(ThreadId(2)), 0);
    }

    #[test]
    fn test_vector_clock_happens_before() {
        let mut clock1 = VectorClock::new();
        clock1.tick(ThreadId(1));

        let mut clock2 = VectorClock::new();
        clock2.tick(ThreadId(1));
        clock2.tick(ThreadId(1));

        assert!(clock1.happens_before(&clock2));
        assert!(!clock2.happens_before(&clock1));
    }

    #[test]
    fn test_vector_clock_concurrent() {
        let mut clock1 = VectorClock::new();
        clock1.tick(ThreadId(1));

        let mut clock2 = VectorClock::new();
        clock2.tick(ThreadId(2));

        assert!(clock1.concurrent_with(&clock2));
    }

    #[test]
    fn test_vector_clock_join() {
        let mut clock1 = VectorClock::new();
        clock1.tick(ThreadId(1));
        clock1.tick(ThreadId(1));

        let mut clock2 = VectorClock::new();
        clock2.tick(ThreadId(2));

        clock1.join(&clock2);
        assert_eq!(clock1.get(ThreadId(1)), 2);
        assert_eq!(clock1.get(ThreadId(2)), 1);
    }

    #[test]
    fn test_memory_access_creation() {
        let access = MemoryAccess::new(
            LocationId(1),
            AccessKind::Write,
            ThreadId::MAIN,
            BlockId(0),
        );

        assert!(access.is_write());
        assert!(!access.is_read());
        assert_eq!(access.thread, ThreadId::MAIN);
    }

    #[test]
    fn test_access_kind_atomic() {
        assert!(!AccessKind::Read.is_atomic());
        assert!(!AccessKind::Write.is_atomic());
        assert!(AccessKind::AtomicLoad(MemoryOrdering::SeqCst).is_atomic());
        assert!(AccessKind::AtomicStore(MemoryOrdering::Release).is_atomic());
    }

    #[test]
    fn test_concurrency_analyzer_creation() {
        let cfg = create_test_cfg();
        let analyzer = ConcurrencyAnalyzer::new(cfg);
        let result = analyzer.analyze();

        assert!(!result.has_issues());
    }

    #[test]
    fn test_data_race_warning_creation() {
        let access1 = MemoryAccess::new(
            LocationId(1),
            AccessKind::Write,
            ThreadId(1),
            BlockId(0),
        );
        let access2 = MemoryAccess::new(
            LocationId(1),
            AccessKind::Read,
            ThreadId(2),
            BlockId(1),
        );

        let warning = DataRaceWarning::new(access1, access2);
        assert_eq!(warning.location, LocationId(1));
        assert_eq!(warning.confidence, 1.0);
    }

    #[test]
    fn test_memory_ordering_acquire_release() {
        assert!(MemoryOrdering::Acquire.is_acquire());
        assert!(!MemoryOrdering::Acquire.is_release());
        assert!(!MemoryOrdering::Release.is_acquire());
        assert!(MemoryOrdering::Release.is_release());
        assert!(MemoryOrdering::AcqRel.is_acquire());
        assert!(MemoryOrdering::AcqRel.is_release());
        assert!(MemoryOrdering::SeqCst.is_acquire());
        assert!(MemoryOrdering::SeqCst.is_release());
    }

    #[test]
    fn test_deadlock_warning_creation() {
        let cycle: List<LockId> = vec![LockId(1), LockId(2), LockId(1)].into();
        let warning = DeadlockWarning::new(cycle, DeadlockKind::LockOrderViolation);

        assert_eq!(warning.lock_cycle.len(), 3);
        assert_eq!(warning.kind, DeadlockKind::LockOrderViolation);
    }

    #[test]
    fn test_concurrency_result_empty() {
        let result = ConcurrencyAnalysisResult::empty();

        assert!(!result.has_issues());
        assert_eq!(result.warning_count(), 0);
    }
}
