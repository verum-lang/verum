//! GPU Race Condition Detection using Z3
//!
//! This module implements data race detection for GPU kernels using SMT solving.
//! A data race occurs when:
//! 1. Two accesses to the same memory location
//! 2. At least one access is a write
//! 3. Accesses are not ordered by happens-before relation
//!
//! ## Happens-Before Relations
//!
//! - **Program Order**: Within a thread, operations are ordered
//! - **Barrier Synchronization**: Barrier creates happens-before edge
//! - **Block Scope**: Threads in different blocks don't synchronize (except global barriers)
//!
//! ## Z3 Encoding Strategy
//!
//! - Memory accesses: Symbolic addresses and values
//! - Happens-before: Partial order constraints
//! - Race condition: SAT query for conflicting accesses
//!
//! ## Performance
//!
//! - Per-access overhead: O(1) encoding
//! - Race detection: O(n²) where n = number of accesses (with optimizations)
//! - Parallel checking: Multiple Z3 solvers for large kernels
//!
//! GPU data race detection for Verum's `@gpu` annotated kernels.
//! Encodes memory accesses as SMT constraints and checks for conflicting accesses
//! without proper synchronization barriers.
//! Based on: GPUVerify (Microsoft Research), GKLEE, PUG

use std::collections::HashMap;
use std::time::Instant;

use z3::{
    SatResult, Solver,
    ast::{Bool, Int},
};

use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;

use crate::gpu_memory_model::{BlockId, MemoryAccess, ThreadId};

// ==================== Core Types ====================

/// Race condition detector using Z3 SMT solver
pub struct RaceDetector {
    /// Memory accesses to check
    accesses: List<MemoryAccess>,

    /// Happens-before graph (adjacency list)
    /// Maps access index to list of indices that happen-before it
    happens_before: Map<usize, Set<usize>>,

    /// Barrier points in the kernel
    barriers: List<BarrierPoint>,

    /// Detected race conditions
    races: List<RaceCondition>,

    /// Statistics
    stats: RaceDetectionStats,
}

impl RaceDetector {
    /// Create a new race detector
    ///
    /// Race detection works from per-access thread/block tags rather than
    /// re-deriving kernel geometry, so dimensions are not retained.
    pub fn new() -> Self {
        Self {
            accesses: List::new(),
            happens_before: Map::new(),
            barriers: List::new(),
            races: List::new(),
            stats: RaceDetectionStats::default(),
        }
    }

    /// Add a memory access
    pub fn add_access(&mut self, access: MemoryAccess) {
        self.accesses.push(access);
    }

    /// Add multiple memory accesses
    pub fn add_accesses(&mut self, accesses: &[MemoryAccess]) {
        for access in accesses {
            self.accesses.push(access.clone());
        }
    }

    /// Add a barrier point
    pub fn add_barrier(&mut self, barrier: BarrierPoint) {
        self.barriers.push(barrier);
    }

    /// Build happens-before graph from program structure
    ///
    /// This encodes:
    /// 1. Program order within each thread
    /// 2. Barrier synchronization within each block
    /// 3. No synchronization between different blocks
    pub fn build_happens_before_graph(&mut self) {
        let start = Instant::now();

        // Group accesses by thread
        let mut thread_accesses: HashMap<(ThreadId, BlockId), Vec<usize>> = HashMap::new();

        for (idx, access) in self.accesses.iter().enumerate() {
            thread_accesses
                .entry((access.thread, access.block))
                .or_default()
                .push(idx);
        }

        // Add program order edges within each thread
        for ((_thread, _block), indices) in &thread_accesses {
            for i in 0..indices.len() {
                for j in i + 1..indices.len() {
                    let earlier = indices[i];
                    let later = indices[j];

                    // Add edge: earlier happens-before later
                    self.happens_before
                        .entry(later)
                        .or_default()
                        .insert(earlier);
                }
            }
        }

        // Add barrier synchronization edges
        let barriers_clone = self.barriers.clone();
        for barrier in &barriers_clone {
            self.add_barrier_edges(barrier, &thread_accesses);
        }

        // Compute transitive closure
        self.compute_transitive_closure();

        self.stats.happens_before_build_time_ms = start.elapsed().as_millis() as u64;
    }

    /// Add happens-before edges for a barrier
    fn add_barrier_edges(
        &mut self,
        barrier: &BarrierPoint,
        thread_accesses: &HashMap<(ThreadId, BlockId), Vec<usize>>,
    ) {
        // Collect all accesses before and after the barrier in the same block
        let mut before_barrier = Vec::new();
        let mut after_barrier = Vec::new();

        for ((thread, block), indices) in thread_accesses {
            // Skip threads in different blocks
            if *block != barrier.block {
                continue;
            }

            for &idx in indices {
                let access = &self.accesses[idx];
                if access.timestamp < barrier.program_point {
                    before_barrier.push(idx);
                } else if access.timestamp >= barrier.program_point {
                    after_barrier.push(idx);
                }
            }
        }

        // All accesses before barrier happen-before all accesses after barrier
        for &before_idx in &before_barrier {
            for &after_idx in &after_barrier {
                self.happens_before
                    .entry(after_idx)
                    .or_default()
                    .insert(before_idx);
            }
        }
    }

    /// Compute transitive closure of happens-before relation
    ///
    /// Uses Floyd-Warshall algorithm: O(n³) but ensures completeness
    fn compute_transitive_closure(&mut self) {
        let n = self.accesses.len();

        // Convert to adjacency matrix for easier computation
        let mut reachable = vec![vec![false; n]; n];

        // Initialize with direct edges
        for (to, froms) in &self.happens_before {
            for from in froms.iter() {
                reachable[*from][*to] = true;
            }
        }

        // Floyd-Warshall
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    if reachable[i][k] && reachable[k][j] {
                        reachable[i][j] = true;
                    }
                }
            }
        }

        // Convert back to map representation
        self.happens_before.clear();
        for j in 0..n {
            let mut predecessors = Set::new();
            for i in 0..n {
                if reachable[i][j] {
                    predecessors.insert(i);
                }
            }
            if !predecessors.is_empty() {
                self.happens_before.insert(j, predecessors);
            }
        }
    }

    /// Encode happens-before relationship between two accesses
    pub fn encode_happens_before(&self, a_idx: usize, b_idx: usize) -> Bool {
        // Check if a happens-before b in the graph
        if let Maybe::Some(predecessors) = self.happens_before.get(&b_idx)
            && predecessors.contains(&a_idx)
        {
            return Bool::from_bool(true);
        }
        Bool::from_bool(false)
    }

    /// Check if two accesses may race
    ///
    /// Returns true if there exists a race condition between accesses a and b
    pub fn check_race(&mut self, a_idx: usize, b_idx: usize) -> bool {
        let start = Instant::now();
        self.stats.total_checks += 1;

        let access_a = &self.accesses[a_idx];
        let access_b = &self.accesses[b_idx];

        // Fast path: Same thread cannot race with itself
        if access_a.thread == access_b.thread && access_a.block == access_b.block {
            return false;
        }

        // Fast path: Different memory spaces cannot race
        if access_a.space != access_b.space {
            return false;
        }

        // Fast path: Two reads cannot race
        if !access_a.is_write && !access_b.is_write {
            return false;
        }

        // Create Z3 solver for this check
        let solver = Solver::new();

        // Constraint 1: Same address
        let addr_a = Int::new_const(access_a.address.as_str());
        let addr_b = Int::new_const(access_b.address.as_str());
        let same_addr = addr_a
            .safe_eq(&addr_b)
            .unwrap_or_else(|_| Bool::from_bool(false));

        // Constraint 2: At least one write (already checked above)
        let conflict = Bool::from_bool(true);

        // Constraint 3: Not ordered by happens-before
        let a_before_b = self.encode_happens_before(a_idx, b_idx);
        let b_before_a = self.encode_happens_before(b_idx, a_idx);
        let not_ordered = Bool::and(&[&a_before_b.not(), &b_before_a.not()]);

        // Race condition = same_addr ∧ conflict ∧ not_ordered
        let race_formula = Bool::and(&[&same_addr, &conflict, &not_ordered]);

        solver.assert(&race_formula);

        let has_race = solver.check() == SatResult::Sat;

        if has_race {
            self.stats.races_found += 1;
        }

        self.stats.check_time_ms += start.elapsed().as_millis() as u64;

        has_race
    }

    /// Find all races in the kernel
    ///
    /// Returns list of detected race conditions
    pub fn find_all_races(&mut self) -> List<RaceCondition> {
        let start = Instant::now();

        // Build happens-before graph first
        self.build_happens_before_graph();

        let n = self.accesses.len();

        // Check all pairs of accesses
        for i in 0..n {
            for j in i + 1..n {
                if self.check_race(i, j) {
                    let race = RaceCondition {
                        access1: self.accesses[i].clone(),
                        access2: self.accesses[j].clone(),
                        access1_idx: i,
                        access2_idx: j,
                        race_type: self.classify_race(&self.accesses[i], &self.accesses[j]),
                    };
                    self.races.push(race);
                }
            }
        }

        self.stats.total_time_ms = start.elapsed().as_millis() as u64;

        self.races.clone()
    }

    /// Classify the type of race condition
    fn classify_race(&self, a: &MemoryAccess, b: &MemoryAccess) -> RaceType {
        if a.is_write && b.is_write {
            RaceType::WriteWrite
        } else if a.is_write || b.is_write {
            RaceType::ReadWrite
        } else {
            RaceType::ReadRead // Should not happen due to fast path
        }
    }

    /// Get detected races
    pub fn get_races(&self) -> &List<RaceCondition> {
        &self.races
    }

    /// Get statistics
    pub fn stats(&self) -> &RaceDetectionStats {
        &self.stats
    }

    /// Clear all state
    pub fn clear(&mut self) {
        self.accesses.clear();
        self.happens_before.clear();
        self.barriers.clear();
        self.races.clear();
        self.stats = RaceDetectionStats::default();
    }
}

/// Barrier synchronization point
#[derive(Debug, Clone)]
pub struct BarrierPoint {
    /// Block containing this barrier
    pub block: BlockId,
    /// Program point (timestamp) of the barrier
    pub program_point: usize,
    /// Barrier ID (for multiple barriers)
    pub barrier_id: u32,
}

/// Detected race condition
#[derive(Debug, Clone)]
pub struct RaceCondition {
    /// First access involved in race
    pub access1: MemoryAccess,
    /// Second access involved in race
    pub access2: MemoryAccess,
    /// Index of first access
    pub access1_idx: usize,
    /// Index of second access
    pub access2_idx: usize,
    /// Type of race
    pub race_type: RaceType,
}

impl RaceCondition {
    /// Format race condition for display
    pub fn format(&self) -> Text {
        format!(
            "Race detected: {:?} between thread {:?} in block {:?} and thread {:?} in block {:?}\n\
             Access 1: {} {:?} @ {} (timestamp {})\n\
             Access 2: {} {:?} @ {} (timestamp {})",
            self.race_type,
            self.access1.thread,
            self.access1.block,
            self.access2.thread,
            self.access2.block,
            if self.access1.is_write {
                "WRITE"
            } else {
                "READ"
            },
            self.access1.space,
            self.access1.address,
            self.access1.timestamp,
            if self.access2.is_write {
                "WRITE"
            } else {
                "READ"
            },
            self.access2.space,
            self.access2.address,
            self.access2.timestamp,
        )
        .to_text()
    }
}

/// Type of race condition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaceType {
    /// Write-Write race (most severe)
    WriteWrite,
    /// Read-Write race
    ReadWrite,
    /// Read-Read (not actually a race, for completeness)
    ReadRead,
}

/// Race detection statistics
#[derive(Debug, Clone, Default)]
pub struct RaceDetectionStats {
    /// Total race checks performed
    pub total_checks: usize,
    /// Races found
    pub races_found: usize,
    /// Time spent building happens-before graph (ms)
    pub happens_before_build_time_ms: u64,
    /// Time spent checking races (ms)
    pub check_time_ms: u64,
    /// Total time (ms)
    pub total_time_ms: u64,
}

impl RaceDetectionStats {
    /// Get race rate (percentage of checks that found races)
    pub fn race_rate(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        (self.races_found as f64 / self.total_checks as f64) * 100.0
    }

    /// Get average check time (ms)
    pub fn avg_check_time_ms(&self) -> f64 {
        if self.total_checks == 0 {
            return 0.0;
        }
        self.check_time_ms as f64 / self.total_checks as f64
    }
}

// ==================== Utilities ====================

/// Create a symbolic happens-before relation
///
/// For advanced verification, you can use symbolic happens-before edges
/// and let Z3 infer the ordering.
pub fn create_symbolic_happens_before(name: &str) -> Bool {
    Bool::new_const(name)
}

/// Encode thread equality constraint
pub fn encode_same_thread(t1: ThreadId, t2: ThreadId, block_dim: (u32, u32, u32)) -> Bool {
    let tid1 = t1.to_z3(block_dim);
    let tid2 = t2.to_z3(block_dim);
    tid1.safe_eq(&tid2)
        .unwrap_or_else(|_| Bool::from_bool(false))
}

/// Encode block equality constraint
pub fn encode_same_block(b1: BlockId, b2: BlockId, grid_dim: (u32, u32, u32)) -> Bool {
    let bid1 = b1.to_z3(grid_dim);
    let bid2 = b2.to_z3(grid_dim);
    bid1.safe_eq(&bid2)
        .unwrap_or_else(|_| Bool::from_bool(false))
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_memory_model::MemorySpace;

    #[test]
    fn test_race_detector_creation() {
        let detector = RaceDetector::new();
        assert_eq!(detector.accesses.len(), 0);
        assert_eq!(detector.races.len(), 0);
    }

    #[test]
    fn test_same_thread_no_race() {
        let mut detector = RaceDetector::new();

        let thread = ThreadId::new(0, 0, 0);
        let block = BlockId::new(0, 0, 0);

        let access1 = MemoryAccess {
            space: MemorySpace::Global,
            thread,
            block,
            address: "addr0".to_text(),
            is_write: true,
            value: Maybe::Some("42".to_text()),
            timestamp: 0,
        };

        let access2 = MemoryAccess {
            space: MemorySpace::Global,
            thread,
            block,
            address: "addr0".to_text(),
            is_write: false,
            value: Maybe::None,
            timestamp: 1,
        };

        detector.add_access(access1);
        detector.add_access(access2);

        // Same thread - no race (program order)
        assert!(!detector.check_race(0, 1));
    }

    #[test]
    fn test_two_reads_no_race() {
        let mut detector = RaceDetector::new();

        let thread1 = ThreadId::new(0, 0, 0);
        let thread2 = ThreadId::new(1, 0, 0);
        let block = BlockId::new(0, 0, 0);

        let access1 = MemoryAccess {
            space: MemorySpace::Global,
            thread: thread1,
            block,
            address: "addr0".to_text(),
            is_write: false,
            value: Maybe::None,
            timestamp: 0,
        };

        let access2 = MemoryAccess {
            space: MemorySpace::Global,
            thread: thread2,
            block,
            address: "addr0".to_text(),
            is_write: false,
            value: Maybe::None,
            timestamp: 1,
        };

        detector.add_access(access1);
        detector.add_access(access2);

        // Two reads - no race
        assert!(!detector.check_race(0, 1));
    }
}
