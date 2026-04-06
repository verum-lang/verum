//! Comprehensive coverage tracking for fuzzing
//!
//! This module provides multiple coverage tracking mechanisms for guiding
//! the fuzzer towards unexplored code paths and compiler behaviors.
//!
//! # Coverage Types
//!
//! - **Line Coverage**: Track which source lines are executed
//! - **Branch Coverage**: Track which branches are taken (true/false paths)
//! - **Edge Coverage**: Track control flow transitions between basic blocks
//! - **AST Node Coverage**: Track which AST node types are exercised
//! - **Error Code Coverage**: Track which error codes are triggered
//! - **SMT Theory Coverage**: Track which SMT theories are exercised
//!
//! # Coverage Model
//!
//! We track coverage using:
//! - **Edge coverage**: Transitions between basic blocks
//! - **Hit counts**: Bucketed counts for loop iterations
//! - **Context sensitivity**: Call-stack aware coverage
//!
//! # Integration
//!
//! Coverage can be collected via:
//! - Compiler instrumentation (preferred for native code)
//! - Interpreter hooks (for Tier 0 execution)
//! - Source-level tracking (for debugging)
//! - Lexer/Parser hooks (for AST coverage)
//! - Error handler hooks (for error coverage)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

/// Size of the coverage bitmap (must be power of 2)
pub const COVERAGE_MAP_SIZE: usize = 65536;

/// Hit count buckets (AFL-style)
const HIT_BUCKETS: [u8; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// Bucketed hit count for a given raw count
fn bucket_hit_count(count: u64) -> u8 {
    match count {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4..=7 => 4,
        8..=15 => 8,
        16..=31 => 16,
        32..=127 => 32,
        _ => 128,
    }
}

/// Edge identifier combining source and destination
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeId {
    /// Source location/block hash
    pub source: u32,
    /// Destination location/block hash
    pub dest: u32,
}

impl EdgeId {
    /// Create a new edge ID
    pub fn new(source: u32, dest: u32) -> Self {
        Self { source, dest }
    }

    /// Compute the coverage map index for this edge
    #[inline]
    pub fn map_index(&self) -> usize {
        // XOR folding to fit in coverage map
        let combined = (self.source as usize) ^ ((self.dest as usize) << 1);
        combined % COVERAGE_MAP_SIZE
    }
}

/// Coverage bitmap for a single execution
pub struct CoverageBitmap {
    /// Raw coverage data
    data: Vec<AtomicU64>,
    /// Number of unique edges hit
    edge_count: AtomicUsize,
}

impl Clone for CoverageBitmap {
    fn clone(&self) -> Self {
        let mut data = Vec::with_capacity(COVERAGE_MAP_SIZE);
        for i in 0..COVERAGE_MAP_SIZE {
            data.push(AtomicU64::new(self.data[i].load(Ordering::Relaxed)));
        }
        Self {
            data,
            edge_count: AtomicUsize::new(self.edge_count.load(Ordering::Relaxed)),
        }
    }
}

impl Default for CoverageBitmap {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverageBitmap {
    /// Create a new empty coverage bitmap
    pub fn new() -> Self {
        let mut data = Vec::with_capacity(COVERAGE_MAP_SIZE);
        for _ in 0..COVERAGE_MAP_SIZE {
            data.push(AtomicU64::new(0));
        }
        Self {
            data,
            edge_count: AtomicUsize::new(0),
        }
    }

    /// Reset the bitmap
    pub fn reset(&self) {
        for i in 0..COVERAGE_MAP_SIZE {
            self.data[i].store(0, Ordering::Relaxed);
        }
        self.edge_count.store(0, Ordering::Relaxed);
    }

    /// Record an edge hit
    #[inline]
    pub fn record_edge(&self, edge: EdgeId) {
        let idx = edge.map_index();
        let prev = self.data[idx].fetch_add(1, Ordering::Relaxed);
        if prev == 0 {
            self.edge_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a hit at a specific index
    #[inline]
    pub fn record_at(&self, index: usize) {
        if index < COVERAGE_MAP_SIZE {
            let prev = self.data[index].fetch_add(1, Ordering::Relaxed);
            if prev == 0 {
                self.edge_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Get the hit count at an index
    pub fn get(&self, index: usize) -> u64 {
        if index < COVERAGE_MAP_SIZE {
            self.data[index].load(Ordering::Relaxed)
        } else {
            0
        }
    }

    /// Get bucketed hit count at index
    pub fn get_bucketed(&self, index: usize) -> u8 {
        bucket_hit_count(self.get(index))
    }

    /// Get the number of unique edges hit
    pub fn edge_count(&self) -> usize {
        self.edge_count.load(Ordering::Relaxed)
    }

    /// Compute a hash of the coverage pattern
    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        for i in 0..COVERAGE_MAP_SIZE {
            let val = self.get_bucketed(i);
            if val > 0 {
                hasher.update(&[i as u8, (i >> 8) as u8, val]);
            }
        }
        let hash = hasher.finalize();
        hex::encode(&hash[..16])
    }

    /// Check if this bitmap has new coverage compared to global
    pub fn has_new_coverage(&self, global: &GlobalCoverage) -> bool {
        for i in 0..COVERAGE_MAP_SIZE {
            let local = self.get_bucketed(i);
            if local > 0 && local > global.get_bucketed(i) {
                return true;
            }
        }
        false
    }

    /// Get indices of new edges compared to global coverage
    pub fn new_edges(&self, global: &GlobalCoverage) -> Vec<usize> {
        let mut edges = Vec::new();
        for i in 0..COVERAGE_MAP_SIZE {
            let local = self.get_bucketed(i);
            if local > 0 && local > global.get_bucketed(i) {
                edges.push(i);
            }
        }
        edges
    }

    /// Convert to a set of covered edge indices
    pub fn to_edge_set(&self) -> HashSet<usize> {
        let mut set = HashSet::new();
        for i in 0..COVERAGE_MAP_SIZE {
            if self.get(i) > 0 {
                set.insert(i);
            }
        }
        set
    }
}

/// Global coverage state tracking all discovered coverage
pub struct GlobalCoverage {
    /// Maximum bucketed hit counts seen per edge
    data: Vec<AtomicU64>,
    /// Set of all discovered edges
    discovered_edges: RwLock<HashSet<usize>>,
    /// Total coverage percentage
    coverage_pct: AtomicU64,
    /// Count of total edges in the program (if known)
    total_edges: AtomicUsize,
}

impl Default for GlobalCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalCoverage {
    /// Create new global coverage tracker
    pub fn new() -> Self {
        let mut data = Vec::with_capacity(COVERAGE_MAP_SIZE);
        for _ in 0..COVERAGE_MAP_SIZE {
            data.push(AtomicU64::new(0));
        }
        Self {
            data,
            discovered_edges: RwLock::new(HashSet::new()),
            coverage_pct: AtomicU64::new(0),
            total_edges: AtomicUsize::new(0),
        }
    }

    /// Set the total number of edges (for percentage calculation)
    pub fn set_total_edges(&self, count: usize) {
        self.total_edges.store(count, Ordering::SeqCst);
        self.update_coverage_pct();
    }

    /// Get bucketed hit count at index
    pub fn get_bucketed(&self, index: usize) -> u8 {
        if index < COVERAGE_MAP_SIZE {
            bucket_hit_count(self.data[index].load(Ordering::Relaxed))
        } else {
            0
        }
    }

    /// Update global coverage with new execution data
    /// Returns true if new coverage was found
    pub fn update(&self, bitmap: &CoverageBitmap) -> bool {
        let mut found_new = false;

        for i in 0..COVERAGE_MAP_SIZE {
            let local = bitmap.get(i);
            if local > 0 {
                // Update max hit count
                let prev = self.data[i].fetch_max(local, Ordering::Relaxed);
                if prev == 0 || bucket_hit_count(local) > bucket_hit_count(prev) {
                    found_new = true;
                }

                // Track discovered edges
                if prev == 0 {
                    if let Ok(mut edges) = self.discovered_edges.write() {
                        edges.insert(i);
                    }
                }
            }
        }

        if found_new {
            self.update_coverage_pct();
        }

        found_new
    }

    /// Get count of discovered unique edges
    pub fn discovered_count(&self) -> usize {
        self.discovered_edges.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Get coverage percentage
    pub fn coverage_pct(&self) -> f64 {
        f64::from_bits(self.coverage_pct.load(Ordering::Relaxed))
    }

    /// Update coverage percentage
    fn update_coverage_pct(&self) {
        let total = self.total_edges.load(Ordering::Relaxed);
        if total > 0 {
            let discovered = self.discovered_count();
            let pct = (discovered as f64 / total as f64) * 100.0;
            self.coverage_pct.store(pct.to_bits(), Ordering::Relaxed);
        }
    }

    /// Get all discovered edge indices
    pub fn discovered_edges(&self) -> HashSet<usize> {
        self.discovered_edges
            .read()
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Reset all coverage data
    pub fn reset(&self) {
        for i in 0..COVERAGE_MAP_SIZE {
            self.data[i].store(0, Ordering::Relaxed);
        }
        if let Ok(mut edges) = self.discovered_edges.write() {
            edges.clear();
        }
        self.coverage_pct.store(0, Ordering::Relaxed);
    }
}

/// Coverage tracker for an execution
pub struct CoverageTracker {
    /// Per-execution bitmap
    bitmap: CoverageBitmap,
    /// Previous edge for computing transitions
    prev_location: AtomicU64,
    /// Call stack depth for context sensitivity
    call_depth: AtomicUsize,
}

impl Default for CoverageTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverageTracker {
    /// Create a new coverage tracker
    pub fn new() -> Self {
        Self {
            bitmap: CoverageBitmap::new(),
            prev_location: AtomicU64::new(0),
            call_depth: AtomicUsize::new(0),
        }
    }

    /// Reset the tracker for a new execution
    pub fn reset(&self) {
        self.bitmap.reset();
        self.prev_location.store(0, Ordering::Relaxed);
        self.call_depth.store(0, Ordering::Relaxed);
    }

    /// Record a location visit (AFL-style edge tracking)
    #[inline]
    pub fn visit_location(&self, location: u32) {
        let prev = self.prev_location.swap(location as u64, Ordering::Relaxed) as u32;
        let edge = EdgeId::new(prev >> 1, location);
        self.bitmap.record_edge(edge);
    }

    /// Record entering a function (for context sensitivity)
    pub fn enter_function(&self, func_id: u32) {
        self.call_depth.fetch_add(1, Ordering::Relaxed);
        // Mix function ID with call depth for context sensitivity
        let depth = self.call_depth.load(Ordering::Relaxed);
        let location = func_id ^ ((depth as u32) << 16);
        self.visit_location(location);
    }

    /// Record leaving a function
    pub fn leave_function(&self) {
        self.call_depth.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the current coverage bitmap
    pub fn bitmap(&self) -> &CoverageBitmap {
        &self.bitmap
    }

    /// Get edge count for this execution
    pub fn edge_count(&self) -> usize {
        self.bitmap.edge_count()
    }
}

/// Coverage-guided input scheduling with energy-based selection
pub struct CoverageScheduler {
    /// Inputs with their coverage info and energy
    inputs: RwLock<Vec<ScheduledInput>>,
    /// Global coverage state
    global_coverage: Arc<GlobalCoverage>,
    /// Total energy of all inputs
    total_energy: AtomicU64,
}

/// A scheduled input with coverage information
#[derive(Clone, Serialize, Deserialize)]
pub struct ScheduledInput {
    /// Input content hash
    pub hash: String,
    /// Unique edges discovered by this input
    pub unique_edges: usize,
    /// Selection energy (higher = more likely to be picked)
    pub energy: f64,
    /// Times this input was selected
    pub selection_count: usize,
    /// New inputs found by mutating this one
    pub children_found: usize,
    /// Execution time in nanoseconds
    pub exec_time_ns: u64,
    /// Size in bytes
    pub size: usize,
}

impl CoverageScheduler {
    /// Create a new coverage scheduler
    pub fn new(global_coverage: Arc<GlobalCoverage>) -> Self {
        Self {
            inputs: RwLock::new(Vec::new()),
            global_coverage,
            total_energy: AtomicU64::new(0),
        }
    }

    /// Add an input with its coverage information
    pub fn add_input(
        &self,
        hash: String,
        coverage: &CoverageBitmap,
        exec_time_ns: u64,
        size: usize,
    ) -> bool {
        // Check if this input has new coverage
        if !coverage.has_new_coverage(&self.global_coverage) {
            return false;
        }

        // Update global coverage
        self.global_coverage.update(coverage);

        // Calculate initial energy
        let unique_edges = coverage.new_edges(&self.global_coverage).len();
        let energy = Self::calculate_energy(unique_edges, size, exec_time_ns);

        let input = ScheduledInput {
            hash,
            unique_edges,
            energy,
            selection_count: 0,
            children_found: 0,
            exec_time_ns,
            size,
        };

        if let Ok(mut inputs) = self.inputs.write() {
            inputs.push(input);
            self.update_total_energy(&inputs);

            // Prune if over limit to prevent unbounded memory growth
            if inputs.len() > Self::MAX_INPUTS {
                // Sort by energy and keep top entries
                inputs.sort_by(|a, b| {
                    b.energy
                        .partial_cmp(&a.energy)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                inputs.truncate(Self::MAX_INPUTS);
                self.update_total_energy(&inputs);
            }
        }

        true
    }

    /// Select an input for mutation (energy-based selection)
    pub fn select<R: rand::Rng>(&self, rng: &mut R) -> Option<String> {
        let inputs = self.inputs.read().ok()?;
        if inputs.is_empty() {
            return None;
        }

        let total = f64::from_bits(self.total_energy.load(Ordering::Relaxed));
        if total <= 0.0 {
            // Uniform random if no energy
            return Some(inputs[rng.random_range(0..inputs.len())].hash.clone());
        }

        // Weighted random selection based on energy
        let mut choice = rng.random::<f64>() * total;
        for input in inputs.iter() {
            choice -= input.energy;
            if choice <= 0.0 {
                return Some(input.hash.clone());
            }
        }

        // Fallback to last input
        inputs.last().map(|i| i.hash.clone())
    }

    /// Mark an input as selected (decreases energy slightly)
    pub fn mark_selected(&self, hash: &str) {
        if let Ok(mut inputs) = self.inputs.write() {
            for input in inputs.iter_mut() {
                if input.hash == hash {
                    input.selection_count += 1;
                    // Energy decay with selection count
                    input.energy *= 0.99;
                    break;
                }
            }
            self.update_total_energy(&inputs);
        }
    }

    /// Mark an input as productive (found new coverage when mutated)
    pub fn mark_productive(&self, hash: &str) {
        if let Ok(mut inputs) = self.inputs.write() {
            for input in inputs.iter_mut() {
                if input.hash == hash {
                    input.children_found += 1;
                    // Boost energy for productive inputs
                    input.energy *= 1.1;
                    break;
                }
            }
            self.update_total_energy(&inputs);
        }
    }

    /// Calculate energy for an input
    fn calculate_energy(unique_edges: usize, size: usize, exec_time_ns: u64) -> f64 {
        let base = 1.0;

        // Prefer inputs with more unique coverage
        let coverage_bonus = (unique_edges as f64) * 0.5;

        // Prefer smaller inputs
        let size_bonus = 100.0 / (size as f64 + 1.0);

        // Prefer faster inputs
        let speed_bonus = 1_000_000.0 / (exec_time_ns as f64 + 1.0);

        base + coverage_bonus + size_bonus + speed_bonus
    }

    /// Update total energy
    fn update_total_energy(&self, inputs: &[ScheduledInput]) {
        let total: f64 = inputs.iter().map(|i| i.energy).sum();
        self.total_energy.store(total.to_bits(), Ordering::Relaxed);
    }

    /// Maximum number of inputs to keep (prevents unbounded memory growth)
    const MAX_INPUTS: usize = 10_000;

    /// Prune low-energy inputs when the collection grows too large
    /// This prevents unbounded memory growth in long-running fuzzing sessions
    pub fn prune_low_energy(&self, keep_count: usize) {
        if let Ok(mut inputs) = self.inputs.write() {
            if inputs.len() <= keep_count {
                return;
            }
            // Sort by energy (descending) and keep top entries
            inputs.sort_by(|a, b| {
                b.energy
                    .partial_cmp(&a.energy)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            inputs.truncate(keep_count);
            self.update_total_energy(&inputs);
        }
    }

    /// Reset all state (call between fuzzing campaigns)
    pub fn reset(&self) {
        if let Ok(mut inputs) = self.inputs.write() {
            inputs.clear();
        }
        self.total_energy.store(0, Ordering::Relaxed);
        self.global_coverage.reset();
    }

    /// Get the number of scheduled inputs
    pub fn len(&self) -> usize {
        self.inputs.read().map(|i| i.len()).unwrap_or(0)
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get coverage statistics
    pub fn stats(&self) -> CoverageStats {
        let discovered = self.global_coverage.discovered_count();
        let total = self.global_coverage.total_edges.load(Ordering::Relaxed);
        let pct = self.global_coverage.coverage_pct();
        let inputs = self.len();

        CoverageStats {
            discovered_edges: discovered,
            total_edges: total,
            coverage_pct: pct,
            corpus_size: inputs,
        }
    }
}

/// Coverage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CoverageStats {
    /// Number of discovered unique edges
    pub discovered_edges: usize,
    /// Total edges in the program (if known)
    pub total_edges: usize,
    /// Coverage percentage
    pub coverage_pct: f64,
    /// Number of inputs in corpus
    pub corpus_size: usize,
}

/// Source-level coverage tracking for debugging
#[derive(Debug, Default)]
pub struct SourceCoverage {
    /// Files and their covered lines
    files: RwLock<HashMap<String, HashSet<usize>>>,
    /// Total lines discovered
    total_lines: AtomicUsize,
}

impl SourceCoverage {
    /// Create new source coverage tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a line being executed
    pub fn record_line(&self, file: &str, line: usize) {
        if let Ok(mut files) = self.files.write() {
            let entry = files.entry(file.to_string()).or_default();
            if entry.insert(line) {
                self.total_lines.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Get covered lines for a file
    pub fn get_file_coverage(&self, file: &str) -> Option<HashSet<usize>> {
        self.files.read().ok()?.get(file).cloned()
    }

    /// Get total covered lines
    pub fn total_lines(&self) -> usize {
        self.total_lines.load(Ordering::Relaxed)
    }

    /// Generate coverage report
    pub fn report(&self) -> SourceCoverageReport {
        let files = self.files.read().unwrap();
        let file_coverage: HashMap<String, Vec<usize>> = files
            .iter()
            .map(|(file, lines)| {
                let mut sorted: Vec<_> = lines.iter().cloned().collect();
                sorted.sort();
                (file.clone(), sorted)
            })
            .collect();

        SourceCoverageReport {
            files: file_coverage,
            total_lines: self.total_lines.load(Ordering::Relaxed),
        }
    }

    /// Reset all coverage state (call between fuzzing campaigns to prevent memory growth)
    pub fn reset(&self) {
        if let Ok(mut files) = self.files.write() {
            files.clear();
        }
        self.total_lines.store(0, Ordering::Relaxed);
    }
}

/// Source coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCoverageReport {
    /// Files and their covered lines (sorted)
    pub files: HashMap<String, Vec<usize>>,
    /// Total covered lines
    pub total_lines: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_id() {
        let edge = EdgeId::new(100, 200);
        let idx = edge.map_index();
        assert!(idx < COVERAGE_MAP_SIZE);
    }

    #[test]
    fn test_coverage_bitmap() {
        let bitmap = CoverageBitmap::new();
        assert_eq!(bitmap.edge_count(), 0);

        bitmap.record_edge(EdgeId::new(1, 2));
        assert_eq!(bitmap.edge_count(), 1);

        bitmap.record_edge(EdgeId::new(1, 2));
        assert_eq!(bitmap.edge_count(), 1); // Same edge, count stays 1

        bitmap.record_edge(EdgeId::new(2, 3));
        assert_eq!(bitmap.edge_count(), 2);
    }

    #[test]
    fn test_global_coverage() {
        let global = GlobalCoverage::new();
        let bitmap = CoverageBitmap::new();

        bitmap.record_edge(EdgeId::new(1, 2));
        bitmap.record_edge(EdgeId::new(2, 3));

        let found_new = global.update(&bitmap);
        assert!(found_new);
        assert_eq!(global.discovered_count(), 2);

        // Same coverage again
        let found_new = global.update(&bitmap);
        assert!(!found_new); // No new coverage
    }

    #[test]
    fn test_coverage_tracker() {
        let tracker = CoverageTracker::new();

        tracker.visit_location(100);
        tracker.visit_location(200);
        tracker.visit_location(300);

        // Should have recorded edges: 0->100, 50->200, 100->300
        assert!(tracker.edge_count() > 0);
    }

    #[test]
    fn test_hit_buckets() {
        assert_eq!(bucket_hit_count(0), 0);
        assert_eq!(bucket_hit_count(1), 1);
        assert_eq!(bucket_hit_count(2), 2);
        assert_eq!(bucket_hit_count(3), 3);
        assert_eq!(bucket_hit_count(5), 4);
        assert_eq!(bucket_hit_count(10), 8);
        assert_eq!(bucket_hit_count(20), 16);
        assert_eq!(bucket_hit_count(50), 32);
        assert_eq!(bucket_hit_count(200), 128);
    }

    #[test]
    fn test_source_coverage() {
        let coverage = SourceCoverage::new();

        coverage.record_line("test.vr", 1);
        coverage.record_line("test.vr", 2);
        coverage.record_line("test.vr", 1); // Duplicate

        assert_eq!(coverage.total_lines(), 2);

        let file_cov = coverage.get_file_coverage("test.vr").unwrap();
        assert!(file_cov.contains(&1));
        assert!(file_cov.contains(&2));
    }
}

// ============================================================================
// Branch Coverage
// ============================================================================

/// Branch coverage tracker
///
/// Tracks which branches (true/false paths of conditionals) are taken.
/// This is more precise than line coverage for control flow.
pub struct BranchCoverage {
    /// Branches and their coverage state
    branches: RwLock<HashMap<BranchId, BranchState>>,
    /// Total branch count
    total_branches: AtomicUsize,
    /// Covered branch count
    covered_branches: AtomicUsize,
}

/// Unique identifier for a branch point
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BranchId {
    /// File path
    pub file: String,
    /// Line number
    pub line: usize,
    /// Column number
    pub column: usize,
    /// Branch description (e.g., "if condition", "match arm 0")
    pub description: String,
}

impl BranchId {
    /// Create a new branch ID
    pub fn new(file: &str, line: usize, column: usize, description: &str) -> Self {
        Self {
            file: file.to_string(),
            line,
            column,
            description: description.to_string(),
        }
    }

    /// Create from source location
    pub fn from_location(file: &str, line: usize, column: usize) -> Self {
        Self::new(file, line, column, "")
    }
}

/// State of a branch
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BranchState {
    /// True path taken
    pub true_taken: bool,
    /// False path taken
    pub false_taken: bool,
    /// Hit counts
    pub true_count: usize,
    pub false_count: usize,
}

impl BranchState {
    /// Check if branch is fully covered
    pub fn is_fully_covered(&self) -> bool {
        self.true_taken && self.false_taken
    }

    /// Get coverage percentage (0, 50, or 100)
    pub fn coverage_pct(&self) -> f64 {
        match (self.true_taken, self.false_taken) {
            (true, true) => 100.0,
            (true, false) | (false, true) => 50.0,
            (false, false) => 0.0,
        }
    }
}

impl Default for BranchCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl BranchCoverage {
    /// Create new branch coverage tracker
    pub fn new() -> Self {
        Self {
            branches: RwLock::new(HashMap::new()),
            total_branches: AtomicUsize::new(0),
            covered_branches: AtomicUsize::new(0),
        }
    }

    /// Register a branch point
    pub fn register_branch(&self, id: BranchId) {
        if let Ok(mut branches) = self.branches.write() {
            if !branches.contains_key(&id) {
                branches.insert(id, BranchState::default());
                self.total_branches.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record a branch taken
    pub fn record_branch(&self, id: &BranchId, taken: bool) {
        if let Ok(mut branches) = self.branches.write() {
            let state = branches.entry(id.clone()).or_default();
            let was_covered = state.is_fully_covered();

            if taken {
                state.true_taken = true;
                state.true_count += 1;
            } else {
                state.false_taken = true;
                state.false_count += 1;
            }

            let now_covered = state.is_fully_covered();
            if !was_covered && now_covered {
                self.covered_branches.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Get branch state
    pub fn get_branch(&self, id: &BranchId) -> Option<BranchState> {
        self.branches.read().ok()?.get(id).cloned()
    }

    /// Get total branch count
    pub fn total_branches(&self) -> usize {
        self.total_branches.load(Ordering::Relaxed)
    }

    /// Get covered branch count (both paths taken)
    pub fn covered_branches(&self) -> usize {
        self.covered_branches.load(Ordering::Relaxed)
    }

    /// Get coverage percentage
    pub fn coverage_pct(&self) -> f64 {
        let total = self.total_branches();
        if total == 0 {
            0.0
        } else {
            (self.covered_branches() as f64 / total as f64) * 100.0
        }
    }

    /// Get uncovered branches
    pub fn uncovered_branches(&self) -> Vec<(BranchId, BranchState)> {
        self.branches
            .read()
            .ok()
            .map(|branches| {
                branches
                    .iter()
                    .filter(|(_, state)| !state.is_fully_covered())
                    .map(|(id, state)| (id.clone(), state.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Generate report
    pub fn report(&self) -> BranchCoverageReport {
        let branches = self.branches.read().unwrap();

        BranchCoverageReport {
            total_branches: self.total_branches(),
            covered_branches: self.covered_branches(),
            coverage_pct: self.coverage_pct(),
            by_file: self.group_by_file(&branches),
        }
    }

    /// Reset all coverage state (call between fuzzing campaigns to prevent memory growth)
    pub fn reset(&self) {
        if let Ok(mut branches) = self.branches.write() {
            branches.clear();
        }
        self.total_branches.store(0, Ordering::Relaxed);
        self.covered_branches.store(0, Ordering::Relaxed);
    }

    fn group_by_file(
        &self,
        branches: &HashMap<BranchId, BranchState>,
    ) -> HashMap<String, FileBranchCoverage> {
        let mut by_file: HashMap<String, Vec<(BranchId, BranchState)>> = HashMap::new();

        for (id, state) in branches {
            by_file
                .entry(id.file.clone())
                .or_default()
                .push((id.clone(), state.clone()));
        }

        by_file
            .into_iter()
            .map(|(file, branches)| {
                let total = branches.len();
                let covered = branches
                    .iter()
                    .filter(|(_, s)| s.is_fully_covered())
                    .count();
                let pct = if total > 0 {
                    (covered as f64 / total as f64) * 100.0
                } else {
                    0.0
                };

                (
                    file,
                    FileBranchCoverage {
                        total_branches: total,
                        covered_branches: covered,
                        coverage_pct: pct,
                    },
                )
            })
            .collect()
    }
}

/// Branch coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCoverageReport {
    /// Total branches
    pub total_branches: usize,
    /// Covered branches
    pub covered_branches: usize,
    /// Coverage percentage
    pub coverage_pct: f64,
    /// Coverage by file
    pub by_file: HashMap<String, FileBranchCoverage>,
}

/// File-level branch coverage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBranchCoverage {
    /// Total branches in file
    pub total_branches: usize,
    /// Covered branches in file
    pub covered_branches: usize,
    /// Coverage percentage
    pub coverage_pct: f64,
}

// ============================================================================
// AST Node Coverage
// ============================================================================

/// AST node coverage tracker
///
/// Tracks which AST node types are exercised by the fuzzer.
/// This helps ensure the parser/compiler handles all language constructs.
pub struct AstNodeCoverage {
    /// Node types and their coverage counts
    node_types: RwLock<HashMap<String, AstNodeStats>>,
    /// Total node types seen
    total_types: AtomicUsize,
    /// All possible node types (if known)
    known_types: RwLock<HashSet<String>>,
}

/// Statistics for an AST node type
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AstNodeStats {
    /// Times this node type was seen
    pub count: usize,
    /// First example input that produced this node
    pub first_example: Option<String>,
    /// Maximum depth at which this node was seen
    pub max_depth: usize,
    /// Whether this node was in a valid program
    pub in_valid_program: bool,
    /// Whether this node triggered an error
    pub triggered_error: bool,
}

impl Default for AstNodeCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl AstNodeCoverage {
    /// Create new AST node coverage tracker
    pub fn new() -> Self {
        Self {
            node_types: RwLock::new(HashMap::new()),
            total_types: AtomicUsize::new(0),
            known_types: RwLock::new(HashSet::new()),
        }
    }

    /// Register all known node types
    pub fn register_known_types(&self, types: &[&str]) {
        if let Ok(mut known) = self.known_types.write() {
            for ty in types {
                known.insert(ty.to_string());
            }
        }
    }

    /// Record an AST node being parsed/visited
    pub fn record_node(
        &self,
        node_type: &str,
        depth: usize,
        example: Option<&str>,
        valid: bool,
        errored: bool,
    ) {
        if let Ok(mut nodes) = self.node_types.write() {
            let stats = nodes.entry(node_type.to_string()).or_default();

            if stats.count == 0 {
                self.total_types.fetch_add(1, Ordering::Relaxed);
            }

            stats.count += 1;
            stats.max_depth = stats.max_depth.max(depth);
            stats.in_valid_program = stats.in_valid_program || valid;
            stats.triggered_error = stats.triggered_error || errored;

            if stats.first_example.is_none() {
                stats.first_example = example.map(|s| s.to_string());
            }
        }
    }

    /// Get coverage for a node type
    pub fn get_node(&self, node_type: &str) -> Option<AstNodeStats> {
        self.node_types.read().ok()?.get(node_type).cloned()
    }

    /// Get total covered node types
    pub fn covered_types(&self) -> usize {
        self.total_types.load(Ordering::Relaxed)
    }

    /// Get coverage percentage (if known types are registered)
    pub fn coverage_pct(&self) -> Option<f64> {
        let known = self.known_types.read().ok()?.len();
        if known == 0 {
            return None;
        }

        let covered = self.covered_types();
        Some((covered as f64 / known as f64) * 100.0)
    }

    /// Get uncovered node types
    pub fn uncovered_types(&self) -> Vec<String> {
        let known = self.known_types.read().ok();
        let covered = self.node_types.read().ok();

        match (known, covered) {
            (Some(known), Some(covered)) => known
                .iter()
                .filter(|t| !covered.contains_key(*t))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Generate report
    pub fn report(&self) -> AstNodeCoverageReport {
        let nodes = self.node_types.read().unwrap();
        let known = self.known_types.read().ok().map(|k| k.len()).unwrap_or(0);

        AstNodeCoverageReport {
            covered_types: self.covered_types(),
            known_types: known,
            coverage_pct: self.coverage_pct(),
            by_type: nodes.clone(),
            uncovered: self.uncovered_types(),
        }
    }

    /// Reset all coverage state (call between fuzzing campaigns to prevent memory growth)
    pub fn reset(&self) {
        if let Ok(mut nodes) = self.node_types.write() {
            nodes.clear();
        }
        if let Ok(mut known) = self.known_types.write() {
            known.clear();
        }
        self.total_types.store(0, Ordering::Relaxed);
    }
}

/// AST node coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstNodeCoverageReport {
    /// Number of covered types
    pub covered_types: usize,
    /// Number of known types
    pub known_types: usize,
    /// Coverage percentage
    pub coverage_pct: Option<f64>,
    /// Stats by node type
    pub by_type: HashMap<String, AstNodeStats>,
    /// Uncovered types
    pub uncovered: Vec<String>,
}

// ============================================================================
// Error Code Coverage
// ============================================================================

/// Error code coverage tracker
///
/// Tracks which compiler error codes have been triggered.
/// This helps ensure the error handling paths are exercised.
pub struct ErrorCodeCoverage {
    /// Error codes and their occurrences
    error_codes: RwLock<HashMap<String, ErrorCodeStats>>,
    /// All known error codes
    known_codes: RwLock<HashSet<String>>,
    /// Total unique error codes triggered
    total_triggered: AtomicUsize,
}

/// Statistics for an error code
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorCodeStats {
    /// Times this error was triggered
    pub count: usize,
    /// First input that triggered this error
    pub first_input: Option<String>,
    /// Error message template
    pub message_template: Option<String>,
    /// Severity level
    pub severity: ErrorSeverity,
    /// Whether this was a regression (appeared in newer code)
    pub is_regression: bool,
}

/// Error severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ErrorSeverity {
    /// Hint/suggestion
    Hint,
    /// Warning
    Warning,
    /// Error
    #[default]
    Error,
    /// Fatal/ICE
    Fatal,
}

impl Default for ErrorCodeCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorCodeCoverage {
    /// Create new error code coverage tracker
    pub fn new() -> Self {
        Self {
            error_codes: RwLock::new(HashMap::new()),
            known_codes: RwLock::new(HashSet::new()),
            total_triggered: AtomicUsize::new(0),
        }
    }

    /// Register all known error codes
    pub fn register_known_codes(&self, codes: &[&str]) {
        if let Ok(mut known) = self.known_codes.write() {
            for code in codes {
                known.insert(code.to_string());
            }
        }
    }

    /// Record an error code being triggered
    pub fn record_error(
        &self,
        code: &str,
        input: Option<&str>,
        message: Option<&str>,
        severity: ErrorSeverity,
    ) {
        if let Ok(mut errors) = self.error_codes.write() {
            let stats = errors.entry(code.to_string()).or_default();

            if stats.count == 0 {
                self.total_triggered.fetch_add(1, Ordering::Relaxed);
            }

            stats.count += 1;
            stats.severity = severity;

            if stats.first_input.is_none() {
                stats.first_input = input.map(|s| {
                    // Truncate long inputs
                    if s.len() > 200 {
                        format!("{}...", &s[..200])
                    } else {
                        s.to_string()
                    }
                });
            }

            if stats.message_template.is_none() {
                stats.message_template = message.map(|s| s.to_string());
            }
        }
    }

    /// Get stats for an error code
    pub fn get_error(&self, code: &str) -> Option<ErrorCodeStats> {
        self.error_codes.read().ok()?.get(code).cloned()
    }

    /// Get total triggered error codes
    pub fn triggered_count(&self) -> usize {
        self.total_triggered.load(Ordering::Relaxed)
    }

    /// Get coverage percentage
    pub fn coverage_pct(&self) -> Option<f64> {
        let known = self.known_codes.read().ok()?.len();
        if known == 0 {
            return None;
        }

        let triggered = self.triggered_count();
        Some((triggered as f64 / known as f64) * 100.0)
    }

    /// Get untriggered error codes
    pub fn untriggered_codes(&self) -> Vec<String> {
        let known = self.known_codes.read().ok();
        let triggered = self.error_codes.read().ok();

        match (known, triggered) {
            (Some(known), Some(triggered)) => known
                .iter()
                .filter(|c| !triggered.contains_key(*c))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Generate report
    pub fn report(&self) -> ErrorCodeCoverageReport {
        let errors = self.error_codes.read().unwrap();
        let known = self.known_codes.read().ok().map(|k| k.len()).unwrap_or(0);

        ErrorCodeCoverageReport {
            triggered_count: self.triggered_count(),
            known_count: known,
            coverage_pct: self.coverage_pct(),
            by_code: errors.clone(),
            untriggered: self.untriggered_codes(),
            by_severity: self.group_by_severity(&errors),
        }
    }

    fn group_by_severity(
        &self,
        errors: &HashMap<String, ErrorCodeStats>,
    ) -> HashMap<String, usize> {
        let mut by_severity: HashMap<String, usize> = HashMap::new();

        for stats in errors.values() {
            let key = format!("{:?}", stats.severity);
            *by_severity.entry(key).or_insert(0) += 1;
        }

        by_severity
    }
}

/// Error code coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorCodeCoverageReport {
    /// Triggered error codes count
    pub triggered_count: usize,
    /// Known error codes count
    pub known_count: usize,
    /// Coverage percentage
    pub coverage_pct: Option<f64>,
    /// Stats by error code
    pub by_code: HashMap<String, ErrorCodeStats>,
    /// Untriggered codes
    pub untriggered: Vec<String>,
    /// Count by severity
    pub by_severity: HashMap<String, usize>,
}

// ============================================================================
// SMT Theory Coverage
// ============================================================================

/// SMT theory coverage tracker
///
/// Tracks which SMT solver theories are exercised by verification.
/// This helps ensure the solver integration handles all theory combinations.
pub struct SmtTheoryCoverage {
    /// Theories and their usage
    theories: RwLock<HashMap<SmtTheory, SmtTheoryStats>>,
    /// Theory combinations seen
    combinations: RwLock<HashSet<String>>,
    /// Total unique theories used
    total_theories: AtomicUsize,
}

/// SMT theory type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmtTheory {
    /// Core (propositional logic)
    Core,
    /// Uninterpreted functions
    UF,
    /// Linear integer arithmetic
    LIA,
    /// Linear real arithmetic
    LRA,
    /// Non-linear integer arithmetic
    NIA,
    /// Non-linear real arithmetic
    NRA,
    /// Bitvectors
    BV,
    /// Arrays
    Arrays,
    /// Datatypes (ADTs)
    Datatypes,
    /// Quantifiers
    Quantifiers,
    /// Strings
    Strings,
    /// Floating-point
    FP,
    /// Sequences
    Sequences,
    /// Sets
    Sets,
}

impl SmtTheory {
    /// Get all theories
    pub fn all() -> &'static [SmtTheory] {
        &[
            SmtTheory::Core,
            SmtTheory::UF,
            SmtTheory::LIA,
            SmtTheory::LRA,
            SmtTheory::NIA,
            SmtTheory::NRA,
            SmtTheory::BV,
            SmtTheory::Arrays,
            SmtTheory::Datatypes,
            SmtTheory::Quantifiers,
            SmtTheory::Strings,
            SmtTheory::FP,
            SmtTheory::Sequences,
            SmtTheory::Sets,
        ]
    }

    /// Get short name
    pub fn short_name(&self) -> &'static str {
        match self {
            SmtTheory::Core => "Core",
            SmtTheory::UF => "UF",
            SmtTheory::LIA => "LIA",
            SmtTheory::LRA => "LRA",
            SmtTheory::NIA => "NIA",
            SmtTheory::NRA => "NRA",
            SmtTheory::BV => "BV",
            SmtTheory::Arrays => "A",
            SmtTheory::Datatypes => "DT",
            SmtTheory::Quantifiers => "Q",
            SmtTheory::Strings => "S",
            SmtTheory::FP => "FP",
            SmtTheory::Sequences => "Seq",
            SmtTheory::Sets => "Sets",
        }
    }
}

/// Statistics for an SMT theory
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SmtTheoryStats {
    /// Times this theory was used
    pub count: usize,
    /// Solver successes with this theory
    pub successes: usize,
    /// Solver timeouts with this theory
    pub timeouts: usize,
    /// Solver unknowns with this theory
    pub unknowns: usize,
    /// Average solve time in milliseconds
    pub avg_time_ms: f64,
    /// Maximum solve time in milliseconds
    pub max_time_ms: u64,
}

impl Default for SmtTheoryCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl SmtTheoryCoverage {
    /// Create new SMT theory coverage tracker
    pub fn new() -> Self {
        Self {
            theories: RwLock::new(HashMap::new()),
            combinations: RwLock::new(HashSet::new()),
            total_theories: AtomicUsize::new(0),
        }
    }

    /// Record SMT solver usage
    pub fn record_usage(
        &self,
        theories: &[SmtTheory],
        success: bool,
        timeout: bool,
        unknown: bool,
        time_ms: u64,
    ) {
        // Record individual theories
        if let Ok(mut theory_stats) = self.theories.write() {
            for theory in theories {
                let stats = theory_stats.entry(*theory).or_default();

                if stats.count == 0 {
                    self.total_theories.fetch_add(1, Ordering::Relaxed);
                }

                stats.count += 1;
                if success {
                    stats.successes += 1;
                }
                if timeout {
                    stats.timeouts += 1;
                }
                if unknown {
                    stats.unknowns += 1;
                }

                // Update timing
                let total_time = stats.avg_time_ms * (stats.count - 1) as f64 + time_ms as f64;
                stats.avg_time_ms = total_time / stats.count as f64;
                stats.max_time_ms = stats.max_time_ms.max(time_ms);
            }
        }

        // Record theory combination
        if let Ok(mut combos) = self.combinations.write() {
            let mut theory_names: Vec<_> = theories.iter().map(|t| t.short_name()).collect();
            theory_names.sort();
            combos.insert(theory_names.join("+"));
        }
    }

    /// Get stats for a theory
    pub fn get_theory(&self, theory: SmtTheory) -> Option<SmtTheoryStats> {
        self.theories.read().ok()?.get(&theory).cloned()
    }

    /// Get covered theory count
    pub fn covered_count(&self) -> usize {
        self.total_theories.load(Ordering::Relaxed)
    }

    /// Get coverage percentage
    pub fn coverage_pct(&self) -> f64 {
        let total = SmtTheory::all().len();
        let covered = self.covered_count();
        (covered as f64 / total as f64) * 100.0
    }

    /// Get uncovered theories
    pub fn uncovered_theories(&self) -> Vec<SmtTheory> {
        let covered = self.theories.read().ok();

        match covered {
            Some(covered) => SmtTheory::all()
                .iter()
                .filter(|t| !covered.contains_key(t))
                .cloned()
                .collect(),
            None => SmtTheory::all().to_vec(),
        }
    }

    /// Get theory combinations seen
    pub fn combinations(&self) -> Vec<String> {
        self.combinations
            .read()
            .ok()
            .map(|c| c.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Generate report
    pub fn report(&self) -> SmtTheoryCoverageReport {
        let theories = self.theories.read().unwrap();

        SmtTheoryCoverageReport {
            covered_count: self.covered_count(),
            total_count: SmtTheory::all().len(),
            coverage_pct: self.coverage_pct(),
            by_theory: theories.clone(),
            uncovered: self.uncovered_theories(),
            combinations: self.combinations(),
        }
    }
}

/// SMT theory coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtTheoryCoverageReport {
    /// Covered theory count
    pub covered_count: usize,
    /// Total theory count
    pub total_count: usize,
    /// Coverage percentage
    pub coverage_pct: f64,
    /// Stats by theory
    pub by_theory: HashMap<SmtTheory, SmtTheoryStats>,
    /// Uncovered theories
    pub uncovered: Vec<SmtTheory>,
    /// Theory combinations seen
    pub combinations: Vec<String>,
}

// ============================================================================
// Unified Coverage Tracker
// ============================================================================

/// Unified coverage tracker combining all coverage types
pub struct UnifiedCoverage {
    /// Edge coverage
    pub edge: Arc<GlobalCoverage>,
    /// Branch coverage
    pub branch: Arc<BranchCoverage>,
    /// Source line coverage
    pub line: Arc<SourceCoverage>,
    /// AST node coverage
    pub ast: Arc<AstNodeCoverage>,
    /// Error code coverage
    pub error: Arc<ErrorCodeCoverage>,
    /// SMT theory coverage
    pub smt: Arc<SmtTheoryCoverage>,
}

impl Default for UnifiedCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl UnifiedCoverage {
    /// Create a new unified coverage tracker
    pub fn new() -> Self {
        Self {
            edge: Arc::new(GlobalCoverage::new()),
            branch: Arc::new(BranchCoverage::new()),
            line: Arc::new(SourceCoverage::new()),
            ast: Arc::new(AstNodeCoverage::new()),
            error: Arc::new(ErrorCodeCoverage::new()),
            smt: Arc::new(SmtTheoryCoverage::new()),
        }
    }

    /// Generate unified coverage report
    pub fn report(&self) -> UnifiedCoverageReport {
        UnifiedCoverageReport {
            edge: CoverageStats {
                discovered_edges: self.edge.discovered_count(),
                total_edges: self.edge.total_edges.load(Ordering::Relaxed),
                coverage_pct: self.edge.coverage_pct(),
                corpus_size: 0,
            },
            branch: self.branch.report(),
            line: self.line.report(),
            ast: self.ast.report(),
            error: self.error.report(),
            smt: self.smt.report(),
        }
    }

    /// Get combined coverage percentage
    pub fn combined_pct(&self) -> f64 {
        let mut total = 0.0;
        let mut count = 0;

        // Edge coverage
        let edge_pct = self.edge.coverage_pct();
        if edge_pct > 0.0 {
            total += edge_pct;
            count += 1;
        }

        // Branch coverage
        let branch_pct = self.branch.coverage_pct();
        if branch_pct > 0.0 {
            total += branch_pct;
            count += 1;
        }

        // AST coverage
        if let Some(ast_pct) = self.ast.coverage_pct() {
            total += ast_pct;
            count += 1;
        }

        // Error coverage
        if let Some(error_pct) = self.error.coverage_pct() {
            total += error_pct;
            count += 1;
        }

        // SMT coverage
        total += self.smt.coverage_pct();
        count += 1;

        if count > 0 { total / count as f64 } else { 0.0 }
    }
}

/// Unified coverage report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCoverageReport {
    /// Edge coverage stats
    pub edge: CoverageStats,
    /// Branch coverage report
    pub branch: BranchCoverageReport,
    /// Line coverage report
    pub line: SourceCoverageReport,
    /// AST node coverage report
    pub ast: AstNodeCoverageReport,
    /// Error code coverage report
    pub error: ErrorCodeCoverageReport,
    /// SMT theory coverage report
    pub smt: SmtTheoryCoverageReport,
}

#[cfg(test)]
mod extended_tests {
    use super::*;

    #[test]
    fn test_branch_coverage() {
        let coverage = BranchCoverage::new();
        let id = BranchId::new("test.vr", 10, 5, "if condition");

        coverage.register_branch(id.clone());
        assert_eq!(coverage.total_branches(), 1);
        assert_eq!(coverage.covered_branches(), 0);

        coverage.record_branch(&id, true);
        assert_eq!(coverage.covered_branches(), 0); // Not fully covered

        coverage.record_branch(&id, false);
        assert_eq!(coverage.covered_branches(), 1); // Now fully covered
    }

    #[test]
    fn test_ast_node_coverage() {
        let coverage = AstNodeCoverage::new();
        coverage.register_known_types(&["FunctionDef", "LetBinding", "IfExpr", "Call"]);

        coverage.record_node("FunctionDef", 0, Some("fn main() {}"), true, false);
        coverage.record_node("LetBinding", 1, Some("let x = 1"), true, false);

        assert_eq!(coverage.covered_types(), 2);
        assert!(coverage.coverage_pct().unwrap() < 100.0);

        let uncovered = coverage.uncovered_types();
        assert!(uncovered.contains(&"IfExpr".to_string()));
        assert!(uncovered.contains(&"Call".to_string()));
    }

    #[test]
    fn test_error_code_coverage() {
        let coverage = ErrorCodeCoverage::new();
        coverage.register_known_codes(&["E0001", "E0002", "E0003", "E0004"]);

        coverage.record_error(
            "E0001",
            Some("let x = 1 + \"hello\""),
            Some("type mismatch"),
            ErrorSeverity::Error,
        );
        coverage.record_error(
            "E0002",
            Some("let x"),
            Some("missing value"),
            ErrorSeverity::Error,
        );

        assert_eq!(coverage.triggered_count(), 2);
        assert_eq!(coverage.coverage_pct(), Some(50.0));

        let untriggered = coverage.untriggered_codes();
        assert!(untriggered.contains(&"E0003".to_string()));
    }

    #[test]
    fn test_smt_theory_coverage() {
        let coverage = SmtTheoryCoverage::new();

        coverage.record_usage(
            &[SmtTheory::LIA, SmtTheory::Arrays],
            true,
            false,
            false,
            100,
        );
        coverage.record_usage(&[SmtTheory::BV], false, true, false, 5000);

        assert_eq!(coverage.covered_count(), 3);
        assert!(coverage.coverage_pct() > 0.0);

        let stats = coverage.get_theory(SmtTheory::LIA).unwrap();
        assert_eq!(stats.successes, 1);
    }

    #[test]
    fn test_unified_coverage() {
        let unified = UnifiedCoverage::new();

        // Record some coverage
        unified
            .branch
            .register_branch(BranchId::new("test.vr", 1, 1, ""));
        unified
            .branch
            .record_branch(&BranchId::new("test.vr", 1, 1, ""), true);
        unified
            .branch
            .record_branch(&BranchId::new("test.vr", 1, 1, ""), false);

        unified.line.record_line("test.vr", 1);
        unified.line.record_line("test.vr", 2);

        unified.ast.record_node("FunctionDef", 0, None, true, false);

        let report = unified.report();
        assert!(report.branch.covered_branches > 0);
        assert!(report.line.total_lines > 0);
        assert!(report.ast.covered_types > 0);
    }
}
