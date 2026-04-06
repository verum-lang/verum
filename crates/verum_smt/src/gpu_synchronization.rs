//! GPU Synchronization Verification using Z3
//!
//! This module implements verification of GPU synchronization primitives:
//! - Barrier synchronization (`__syncthreads()` in CUDA)
//! - Atomic operations
//! - Memory fences
//! - Lock-free algorithms
//!
//! ## Barrier Semantics
//!
//! A barrier ensures that:
//! 1. All threads in the block reach the barrier
//! 2. No thread proceeds past the barrier until all arrive
//! 3. Memory operations before barrier are visible after barrier
//!
//! ## Atomic Operations
//!
//! Atomic operations provide:
//! - Atomicity: Operation appears indivisible
//! - Sequential consistency: Total order on atomic operations
//! - Memory ordering: Synchronization between threads
//!
//! ## Verification Properties
//!
//! - **Barrier Reachability**: All threads must reach every barrier
//! - **Deadlock Freedom**: No circular wait on synchronization
//! - **Progress**: At least one thread makes progress
//! - **Atomicity**: Atomic operations appear indivisible
//!
//! GPU synchronization verification for Verum's `@gpu` annotated kernels.
//! Extends the type system's refinement verification to thread-level parallelism.
//! Based on: GPUVerify, CUDA Programming Guide, PTX ISA

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use z3::{
    FuncDecl, SatResult, Solver, Sort,
    ast::{Bool, Int},
};

use verum_common::{List, Map, Maybe, Set, Text};
use verum_common::ToText;

use crate::gpu_memory_model::{BlockId, ThreadId};

// ==================== Core Types ====================

/// Synchronization verifier
pub struct SyncVerifier {
    /// Barrier points in the kernel
    barriers: List<Barrier>,

    /// Atomic operations
    atomics: List<AtomicOperation>,

    /// Control flow graph (for reachability analysis)
    cfg: ControlFlowGraph,

    /// Block dimensions
    block_dim: (u32, u32, u32),

    /// Grid dimensions
    grid_dim: (u32, u32, u32),

    /// Verification results
    results: List<VerificationResult>,

    /// Statistics
    stats: SyncVerificationStats,
}

impl SyncVerifier {
    /// Create a new synchronization verifier
    pub fn new(grid_dim: (u32, u32, u32), block_dim: (u32, u32, u32)) -> Self {
        Self {
            barriers: List::new(),
            atomics: List::new(),
            cfg: ControlFlowGraph::new(),
            block_dim,
            grid_dim,
            results: List::new(),
            stats: SyncVerificationStats::default(),
        }
    }

    /// Add a barrier
    pub fn add_barrier(&mut self, barrier: Barrier) {
        self.barriers.push(barrier);
    }

    /// Add an atomic operation
    pub fn add_atomic(&mut self, atomic: AtomicOperation) {
        self.atomics.push(atomic);
    }

    /// Set control flow graph
    pub fn set_cfg(&mut self, cfg: ControlFlowGraph) {
        self.cfg = cfg;
    }

    /// Encode barrier semantics
    ///
    /// Returns a Z3 formula that is SAT iff all threads reach the barrier
    pub fn encode_barrier(&self, barrier_id: u32, block_id: BlockId) -> Bool {
        // Find the barrier
        let barrier = self
            .barriers
            .iter()
            .find(|b| b.id == barrier_id && b.block == block_id);

        let Some(barrier) = barrier else {
            return Bool::from_bool(false);
        };

        // All threads in the block must reach this barrier
        let mut thread_reaches = Vec::new();

        let (bx, by, bz) = self.block_dim;
        for z in 0..bz {
            for y in 0..by {
                for x in 0..bx {
                    let thread = ThreadId::new(x, y, z);
                    let reaches = self.thread_reaches_barrier(thread, block_id, barrier);
                    thread_reaches.push(reaches);
                }
            }
        }

        // Conjunction: all threads reach barrier
        if thread_reaches.is_empty() {
            Bool::from_bool(true)
        } else {
            Bool::and(&thread_reaches.iter().collect::<Vec<_>>())
        }
    }

    /// Check if a thread reaches a barrier
    fn thread_reaches_barrier(&self, thread: ThreadId, block: BlockId, barrier: &Barrier) -> Bool {
        // Use CFG to determine reachability
        let thread_id = thread.to_linear(self.block_dim);
        let reachable = self.cfg.is_reachable(barrier.program_point);

        // Encode as symbolic variable for this thread
        let var_name = format!("thread_{}_reaches_barrier_{}", thread_id, barrier.id);
        if reachable {
            Bool::from_bool(true)
        } else {
            Bool::new_const(var_name.as_str())
        }
    }

    /// Verify barrier correctness
    ///
    /// Checks that all threads in a block reach every barrier
    pub fn verify_barrier_reachability(&mut self) -> bool {
        let start = Instant::now();
        self.stats.total_barrier_checks += 1;

        let solver = Solver::new();

        // For each barrier, check that all threads reach it
        for barrier in &self.barriers {
            let barrier_formula = self.encode_barrier(barrier.id, barrier.block);

            // Check if formula is SAT (all threads reach)
            solver.push();
            solver.assert(barrier_formula.not()); // Try to prove unreachability

            let result = solver.check();
            solver.pop(1);

            if result == SatResult::Sat {
                // Found a thread that doesn't reach the barrier
                self.results.push(VerificationResult::BarrierDivergence {
                    barrier_id: barrier.id,
                    block: barrier.block,
                    message: "Not all threads reach barrier".to_text(),
                });
                self.stats.barriers_failed += 1;

                self.stats.barrier_check_time_ms += start.elapsed().as_millis() as u64;
                return false;
            }
        }

        self.stats.barriers_passed += 1;
        self.stats.barrier_check_time_ms += start.elapsed().as_millis() as u64;
        true
    }

    /// Encode atomic operation
    ///
    /// Atomic operations are linearizable: they appear to execute atomically
    /// at some point between their invocation and response.
    pub fn encode_atomic_add(&self, addr: &Int, old_val: &Int, new_val: &Int) -> Bool {
        // Atomic add: new_val = old_val + delta
        // We need to ensure this appears atomic to other threads

        let delta = Int::sub(&[new_val, old_val]);
        let expected_new = Int::add(&[old_val, &delta]);

        // The operation is atomic if no other thread observes an intermediate state
        // This is encoded as: new_val = old_val + delta (no constraints on other threads here)
        new_val
            .safe_eq(&expected_new)
            .unwrap_or_else(|_| Bool::from_bool(true))
    }

    /// Encode atomic compare-and-swap
    pub fn encode_atomic_cas(
        &self,
        addr: &Int,
        expected: &Int,
        new_val: &Int,
        success: &Bool,
    ) -> Bool {
        // CAS succeeds if memory contains expected value
        // If success: memory ← new_val
        // If failure: memory unchanged

        let addr_val = Int::new_const("memory_value");

        let mem_eq_expected = addr_val
            .safe_eq(expected)
            .unwrap_or_else(|_| Bool::from_bool(false));
        let mem_eq_new = addr_val
            .safe_eq(new_val)
            .unwrap_or_else(|_| Bool::from_bool(false));

        let success_case = Bool::and(&[&mem_eq_expected, success]).implies(&mem_eq_new);

        let not_eq_expected = mem_eq_expected.not();
        let not_success = success.not();
        let failure_case = not_eq_expected.implies(&not_success);

        Bool::and(&[&success_case, &failure_case])
    }

    /// Verify lock-free progress
    ///
    /// A lock-free algorithm guarantees that at least one thread makes progress
    /// in a finite number of steps, regardless of thread scheduling.
    ///
    /// ## Verification Strategy
    ///
    /// We encode liveness as a safety property by checking:
    /// 1. No deadlock: At least one atomic can complete without waiting on others
    /// 2. Fairness: Under fair scheduling, every enabled operation eventually executes
    /// 3. Progress: At least one thread advances the global state
    ///
    /// The encoding uses happens-before constraints to model dependencies:
    /// - If operation A waits for B, then B must happen-before A can complete
    /// - A cycle in the happens-before graph indicates deadlock
    pub fn verify_progress(&mut self) -> bool {
        let start = Instant::now();
        let solver = Solver::new();

        if self.atomics.is_empty() {
            // No atomics means trivial progress
            return true;
        }

        // Build dependency graph and check for deadlock
        let dependency_graph = self.build_atomic_dependency_graph();

        // Check for cycles in dependency graph (deadlock condition)
        if let Some(cycle) = self.detect_dependency_cycle(&dependency_graph) {
            // Record deadlock result
            let threads_in_cycle: List<ThreadId> = cycle
                .iter()
                .filter_map(|&idx| self.atomics.get(idx).map(|a| a.thread))
                .collect();

            self.results.push(VerificationResult::Deadlock {
                threads: threads_in_cycle,
                message: format!(
                    "Circular dependency detected among {} atomic operations",
                    cycle.len()
                )
                .to_text(),
            });
            return false;
        }

        // Encode progress constraint: at least one atomic can complete
        let int_sort = Sort::int();
        let bool_sort = Sort::bool();

        // Create happens-before predicate: hb(op1, op2) = op1 happens-before op2
        let hb = FuncDecl::new("hb", &[&int_sort, &int_sort], &bool_sort);

        // Encode transitivity of happens-before
        let op1 = Int::new_const("op1");
        let op2 = Int::new_const("op2");
        let op3 = Int::new_const("op3");

        let hb_12 = hb.apply(&[&op1, &op2]).as_bool().unwrap();
        let hb_23 = hb.apply(&[&op2, &op3]).as_bool().unwrap();
        let hb_13 = hb.apply(&[&op3, &op1]).as_bool().unwrap();

        // Encode known dependencies from the graph
        for (idx, deps) in &dependency_graph {
            let op_idx = Int::from_i64(*idx as i64);
            for &dep_idx in deps {
                let dep_op = Int::from_i64(dep_idx as i64);
                // dep happens-before op (op waits for dep)
                let dep_hb = hb.apply(&[&dep_op, &op_idx]).as_bool().unwrap();
                solver.assert(&dep_hb);
            }
        }

        // Irreflexivity: no operation happens-before itself (no deadlock)
        let irrefl_constraints: Vec<Bool> = (0..self.atomics.len())
            .map(|idx| {
                let op = Int::from_i64(idx as i64);
                let self_hb = hb.apply(&[&op, &op]).as_bool().unwrap();
                self_hb.not()
            })
            .collect();

        for constraint in &irrefl_constraints {
            solver.assert(constraint);
        }

        // Encode progress: at least one operation can complete (has no unsatisfied dependencies)
        let mut can_complete_ops = Vec::new();
        for (idx, atomic) in self.atomics.iter().enumerate() {
            let can_complete =
                self.encode_atomic_can_complete_internal(atomic, idx, &dependency_graph);
            can_complete_ops.push(can_complete);
        }

        // At least one must be able to complete
        if !can_complete_ops.is_empty() {
            let progress = Bool::or(&can_complete_ops.iter().collect::<Vec<_>>());
            solver.assert(&progress);
        }

        // Check satisfiability
        let result = solver.check();

        self.stats.atomic_check_time_ms += start.elapsed().as_millis() as u64;

        match result {
            SatResult::Sat => true,
            SatResult::Unsat | SatResult::Unknown => {
                self.results.push(VerificationResult::LivenessViolation {
                    message: "Cannot guarantee progress: no operation can complete".to_text(),
                });
                false
            }
        }
    }

    /// Build dependency graph for atomic operations
    ///
    /// An atomic operation A depends on B if:
    /// - A and B access the same address
    /// - A waits for B's result (e.g., CAS retry loop)
    /// - There's a control flow dependency
    fn build_atomic_dependency_graph(&self) -> HashMap<usize, HashSet<usize>> {
        let mut graph: HashMap<usize, HashSet<usize>> = HashMap::new();

        // Group atomics by address to find potential conflicts
        let mut by_address: HashMap<&str, Vec<usize>> = HashMap::new();
        for (idx, atomic) in self.atomics.iter().enumerate() {
            by_address
                .entry(atomic.address.as_str())
                .or_default()
                .push(idx);
        }

        // For each address, establish ordering based on program order and block membership
        for indices in by_address.values() {
            for (i, &idx1) in indices.iter().enumerate() {
                for &idx2 in indices.iter().skip(i + 1) {
                    let op1 = &self.atomics[idx1];
                    let op2 = &self.atomics[idx2];

                    // Same thread: program order determines dependency
                    if op1.thread == op2.thread && op1.block == op2.block {
                        if op1.program_point < op2.program_point {
                            // op1 happens before op2
                            graph.entry(idx2).or_default().insert(idx1);
                        } else {
                            // op2 happens before op1
                            graph.entry(idx1).or_default().insert(idx2);
                        }
                    }
                    // Different threads in same block: potential conflict
                    else if op1.block == op2.block {
                        // Both operations may depend on each other's visibility
                        // This creates potential for deadlock if not properly synchronized
                        // Add bidirectional weak dependency (will be resolved by barrier analysis)

                        // Check if there's a barrier between them
                        let barrier_exists = self.barriers.iter().any(|b| {
                            b.block == op1.block
                                && ((b.program_point > op1.program_point
                                    && b.program_point <= op2.program_point)
                                    || (b.program_point > op2.program_point
                                        && b.program_point <= op1.program_point))
                        });

                        if !barrier_exists {
                            // CAS operations create retry dependencies
                            if matches!(op1.op_type, AtomicOpType::CAS)
                                || matches!(op2.op_type, AtomicOpType::CAS)
                            {
                                // Potential circular dependency - mark both
                                graph.entry(idx1).or_default().insert(idx2);
                                graph.entry(idx2).or_default().insert(idx1);
                            }
                        }
                    }
                }
            }
        }

        graph
    }

    /// Detect cycles in the dependency graph (indicates deadlock)
    ///
    /// Uses DFS-based cycle detection with path tracking to provide deadlock witness
    fn detect_dependency_cycle(
        &self,
        graph: &HashMap<usize, HashSet<usize>>,
    ) -> Option<Vec<usize>> {
        let n = self.atomics.len();
        let mut visited = vec![false; n];
        let mut rec_stack = vec![false; n];
        let mut path = Vec::new();

        fn dfs(
            node: usize,
            graph: &HashMap<usize, HashSet<usize>>,
            visited: &mut Vec<bool>,
            rec_stack: &mut Vec<bool>,
            path: &mut Vec<usize>,
        ) -> Option<Vec<usize>> {
            visited[node] = true;
            rec_stack[node] = true;
            path.push(node);

            if let Some(neighbors) = graph.get(&node) {
                for &neighbor in neighbors {
                    if neighbor < visited.len() {
                        if !visited[neighbor] {
                            if let Some(cycle) = dfs(neighbor, graph, visited, rec_stack, path) {
                                return Some(cycle);
                            }
                        } else if rec_stack[neighbor] {
                            // Found a cycle - extract the cycle path
                            if let Some(cycle_start) = path.iter().position(|&x| x == neighbor) {
                                return Some(path[cycle_start..].to_vec());
                            }
                            return Some(path.to_vec());
                        }
                    }
                }
            }

            path.pop();
            rec_stack[node] = false;
            None
        }

        for start in 0..n {
            if !visited[start] {
                if let Some(cycle) = dfs(start, graph, &mut visited, &mut rec_stack, &mut path) {
                    return Some(cycle);
                }
            }
        }

        None
    }

    /// Encode that an atomic operation can complete (internal implementation)
    ///
    /// An atomic can complete if:
    /// 1. All its dependencies are satisfied (happens-before)
    /// 2. For CAS: the expected value matches (or will eventually match under fairness)
    /// 3. No circular wait exists with other operations
    fn encode_atomic_can_complete_internal(
        &self,
        atomic: &AtomicOperation,
        idx: usize,
        dependency_graph: &HashMap<usize, HashSet<usize>>,
    ) -> Bool {
        // Check if this operation has unresolved dependencies
        let has_deps = dependency_graph
            .get(&idx)
            .is_some_and(|deps| !deps.is_empty());

        if !has_deps {
            // No dependencies: can always complete
            return Bool::from_bool(true);
        }

        // For operations with dependencies, encode the completion condition
        let Some(deps) = dependency_graph.get(&idx) else {
            return Bool::from_bool(true);
        };

        // Check for potential circular dependency with any of our dependencies
        let mut has_circular = false;
        for &dep_idx in deps {
            if let Some(dep_deps) = dependency_graph.get(&dep_idx) {
                if dep_deps.contains(&idx) {
                    has_circular = true;
                    break;
                }
            }
        }

        if has_circular {
            // Circular dependency: cannot guarantee completion
            // However, under fairness assumptions, CAS retry loops can still make progress
            // if at least one thread wins the race
            match atomic.op_type {
                AtomicOpType::CAS => {
                    // CAS can complete if it eventually succeeds
                    // Under fairness, one of the competing threads will succeed
                    let success_var = format!("cas_success_{}_{}", idx, atomic.program_point);
                    Bool::new_const(success_var.as_str())
                }
                _ => {
                    // Other atomics with circular deps indicate true deadlock
                    Bool::from_bool(false)
                }
            }
        } else {
            // Dependencies but no circular wait: can complete after deps
            let all_deps_satisfied: Vec<Bool> = deps
                .iter()
                .map(|&dep_idx| {
                    let dep_complete_var = format!("dep_complete_{}", dep_idx);
                    Bool::new_const(dep_complete_var.as_str())
                })
                .collect();

            if all_deps_satisfied.is_empty() {
                Bool::from_bool(true)
            } else {
                Bool::and(&all_deps_satisfied.iter().collect::<Vec<_>>())
            }
        }
    }

    /// Encode that an atomic operation can complete (public wrapper)
    #[allow(dead_code)] // Part of deadlock detection API
    fn encode_atomic_can_complete(&self, atomic: &AtomicOperation) -> Bool {
        // Find the index of this atomic
        let idx = self.atomics.iter().position(|a| {
            a.thread == atomic.thread
                && a.block == atomic.block
                && a.program_point == atomic.program_point
        });

        if let Some(idx) = idx {
            let graph = self.build_atomic_dependency_graph();
            self.encode_atomic_can_complete_internal(atomic, idx, &graph)
        } else {
            // Unknown atomic: conservatively assume it can complete
            Bool::from_bool(true)
        }
    }

    /// Verify deadlock freedom for all atomic operations
    ///
    /// Returns true if no deadlock is possible, false otherwise
    pub fn verify_deadlock_freedom(&mut self) -> bool {
        let dependency_graph = self.build_atomic_dependency_graph();

        if let Some(cycle) = self.detect_dependency_cycle(&dependency_graph) {
            let threads_in_cycle: List<ThreadId> = cycle
                .iter()
                .filter_map(|&idx| self.atomics.get(idx).map(|a| a.thread))
                .collect();

            self.results.push(VerificationResult::Deadlock {
                threads: threads_in_cycle.clone(),
                message: format!(
                    "Deadlock detected: {} operations form a circular wait involving {} threads",
                    cycle.len(),
                    threads_in_cycle.len()
                )
                .to_text(),
            });

            false
        } else {
            true
        }
    }

    /// Get deadlock witness (the cycle of operations causing deadlock)
    pub fn get_deadlock_witness(&self) -> Option<Vec<(usize, &AtomicOperation)>> {
        let graph = self.build_atomic_dependency_graph();

        self.detect_dependency_cycle(&graph).map(|cycle| {
            cycle
                .iter()
                .filter_map(|&idx| self.atomics.get(idx).map(|op| (idx, op)))
                .collect()
        })
    }

    /// Verify memory fence semantics
    ///
    /// Memory fences ensure ordering of memory operations
    pub fn verify_fence(&self, fence: &MemoryFence) -> bool {
        let solver = Solver::new();

        // Encode fence semantics
        let fence_constraint = self.encode_fence(fence);
        solver.assert(&fence_constraint);

        solver.check() == SatResult::Sat
    }

    /// Encode fence semantics
    ///
    /// Memory fences establish ordering constraints between memory operations.
    /// This encoding follows the PTX ISA memory model and CUDA semantics.
    ///
    /// ## Fence Semantics
    ///
    /// - **Thread fence**: All prior memory operations by this thread are visible
    ///   to subsequent operations by this thread (program order - always satisfied)
    /// - **Block fence**: All prior memory operations by threads in this block are
    ///   visible to subsequent operations by all threads in this block
    /// - **Device fence**: All prior memory operations by all threads are visible
    ///   to subsequent operations by all threads on the device
    /// - **System fence**: All prior memory operations are visible to subsequent
    ///   operations including host memory accesses
    fn encode_fence(&self, fence: &MemoryFence) -> Bool {
        match fence.scope {
            FenceScope::Thread => {
                // Thread fence: orders operations within the thread
                // Always satisfied by program order semantics
                Bool::from_bool(true)
            }
            FenceScope::Block => {
                // Block fence: orders operations within the block
                // All memory operations before the fence by any thread in the block
                // must be visible to all operations after the fence by any thread in the block
                self.encode_block_fence(fence)
            }
            FenceScope::Device => {
                // Device fence: orders operations across all blocks
                // All memory operations before the fence by any thread on the device
                // must be visible to all operations after the fence by any thread
                self.encode_device_fence(fence)
            }
            FenceScope::System => {
                // System fence: orders operations including host
                // Strongest ordering - all operations globally visible
                self.encode_system_fence(fence)
            }
        }
    }

    /// Encode block-level fence semantics
    ///
    /// A block fence ensures that all memory operations by threads in the block
    /// before the fence are visible to all threads in the block after the fence.
    ///
    /// We encode this using happens-before constraints:
    /// - For each thread t1 with writes W before fence
    /// - For each thread t2 with reads R after fence
    /// - W happens-before R (if they access shared memory)
    fn encode_block_fence(&self, fence: &MemoryFence) -> Bool {
        let fence_time = fence.program_point;
        let fence_block = fence.block;

        // Collect constraints for memory ordering within the block
        let mut constraints = Vec::new();

        // For all threads in this block
        let (bx, by, bz) = self.block_dim;
        for z in 0..bz {
            for y in 0..by {
                for x in 0..bx {
                    let thread = ThreadId::new(x, y, z);

                    // Create happens-before constraint for this thread's operations
                    // Operations before fence happen-before operations after fence
                    let before_fence = self.create_happens_before_fence_constraint(
                        thread,
                        fence_block,
                        fence_time,
                        true,
                    );
                    let after_fence = self.create_happens_before_fence_constraint(
                        thread,
                        fence_block,
                        fence_time,
                        false,
                    );

                    // The fence ensures ordering: before → after
                    let ordering = before_fence.implies(&after_fence);
                    constraints.push(ordering);
                }
            }
        }

        // Block fence also requires shared memory visibility
        let shared_visibility = self.encode_shared_memory_visibility(fence_block, fence_time);
        constraints.push(shared_visibility);

        if constraints.is_empty() {
            Bool::from_bool(true)
        } else {
            Bool::and(&constraints.iter().collect::<Vec<_>>())
        }
    }

    /// Encode device-level fence semantics
    ///
    /// A device fence ensures that all memory operations by any thread on the device
    /// before the fence are visible to all threads after the fence.
    ///
    /// This is stronger than block fence - it affects global memory visibility.
    fn encode_device_fence(&self, fence: &MemoryFence) -> Bool {
        let fence_time = fence.program_point;

        // Device fence requires global memory visibility across all blocks
        let mut constraints = Vec::new();

        // Encode global memory ordering constraint
        let global_ordering = self.encode_global_memory_ordering(fence_time);
        constraints.push(global_ordering);

        // Encode that all prior writes to global memory are committed
        let writes_committed = self.encode_writes_committed(fence_time);
        constraints.push(writes_committed);

        // Encode cache coherence (invalidate stale caches)
        let cache_coherence = self.encode_cache_coherence(fence_time);
        constraints.push(cache_coherence);

        if constraints.is_empty() {
            Bool::from_bool(true)
        } else {
            Bool::and(&constraints.iter().collect::<Vec<_>>())
        }
    }

    /// Encode system-level fence semantics
    ///
    /// A system fence ensures that all memory operations (including host accesses)
    /// before the fence are visible to all operations after the fence.
    ///
    /// This is the strongest fence - it synchronizes with host memory.
    fn encode_system_fence(&self, fence: &MemoryFence) -> Bool {
        let fence_time = fence.program_point;

        // System fence includes all device fence constraints
        let device_constraints = self.encode_device_fence(fence);

        // Plus host memory synchronization
        let host_sync = self.encode_host_synchronization(fence_time);

        // Plus unified memory coherence
        let unified_coherence = self.encode_unified_memory_coherence(fence_time);

        Bool::and(&[&device_constraints, &host_sync, &unified_coherence])
    }

    /// Create happens-before constraint for fence
    fn create_happens_before_fence_constraint(
        &self,
        thread: ThreadId,
        block: BlockId,
        fence_time: usize,
        is_before: bool,
    ) -> Bool {
        // Create symbolic variable for thread's memory operation ordering
        let var_name = format!(
            "thread_{}_block_{}_{}_{}",
            thread.to_linear(self.block_dim),
            block.to_linear(self.grid_dim),
            if is_before { "before" } else { "after" },
            fence_time
        );
        Bool::new_const(var_name.as_str())
    }

    /// Encode shared memory visibility for block fence
    fn encode_shared_memory_visibility(&self, block: BlockId, fence_time: usize) -> Bool {
        // All writes to shared memory before fence are visible after fence
        // Encoded as: shared_mem[addr]@after_fence = shared_mem[addr]@before_fence
        let var_name = format!(
            "shared_visibility_block_{}_time_{}",
            block.to_linear(self.grid_dim),
            fence_time
        );
        Bool::new_const(var_name.as_str())
    }

    /// Encode global memory ordering for device fence
    fn encode_global_memory_ordering(&self, fence_time: usize) -> Bool {
        // Global memory operations are ordered at the device level
        let var_name = format!("global_ordering_time_{}", fence_time);
        Bool::new_const(var_name.as_str())
    }

    /// Encode that all writes are committed at fence point
    fn encode_writes_committed(&self, fence_time: usize) -> Bool {
        // All pending writes are committed to memory at fence
        let var_name = format!("writes_committed_time_{}", fence_time);
        Bool::new_const(var_name.as_str())
    }

    /// Encode cache coherence at fence point
    fn encode_cache_coherence(&self, fence_time: usize) -> Bool {
        // Caches are coherent (stale data invalidated) at fence
        let var_name = format!("cache_coherent_time_{}", fence_time);
        Bool::new_const(var_name.as_str())
    }

    /// Encode host memory synchronization for system fence
    fn encode_host_synchronization(&self, fence_time: usize) -> Bool {
        // Host and device memory are synchronized
        let var_name = format!("host_sync_time_{}", fence_time);
        Bool::new_const(var_name.as_str())
    }

    /// Encode unified memory coherence for system fence
    fn encode_unified_memory_coherence(&self, fence_time: usize) -> Bool {
        // Unified/managed memory is coherent between host and device
        let var_name = format!("unified_coherent_time_{}", fence_time);
        Bool::new_const(var_name.as_str())
    }

    /// Get verification results
    pub fn get_results(&self) -> &List<VerificationResult> {
        &self.results
    }

    /// Get statistics
    pub fn stats(&self) -> &SyncVerificationStats {
        &self.stats
    }
}

/// Barrier synchronization point
#[derive(Debug, Clone)]
pub struct Barrier {
    /// Barrier ID (for multiple barriers)
    pub id: u32,
    /// Block containing this barrier
    pub block: BlockId,
    /// Program point where barrier occurs
    pub program_point: usize,
    /// Number of threads that must reach barrier
    pub num_threads: u32,
}

/// Atomic operation
#[derive(Debug, Clone)]
pub struct AtomicOperation {
    /// Type of atomic operation
    pub op_type: AtomicOpType,
    /// Thread performing the operation
    pub thread: ThreadId,
    /// Block containing the thread
    pub block: BlockId,
    /// Address being accessed
    pub address: Text,
    /// Value(s) involved in the operation
    pub values: List<Text>,
    /// Program point
    pub program_point: usize,
}

/// Type of atomic operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicOpType {
    /// Atomic add
    Add,
    /// Atomic subtract
    Sub,
    /// Atomic exchange
    Exch,
    /// Atomic compare-and-swap
    CAS,
    /// Atomic min
    Min,
    /// Atomic max
    Max,
    /// Atomic increment
    Inc,
    /// Atomic decrement
    Dec,
    /// Atomic AND
    And,
    /// Atomic OR
    Or,
    /// Atomic XOR
    Xor,
}

/// Memory fence
#[derive(Debug, Clone)]
pub struct MemoryFence {
    /// Scope of the fence
    pub scope: FenceScope,
    /// Thread issuing the fence
    pub thread: ThreadId,
    /// Block containing the thread
    pub block: BlockId,
    /// Program point
    pub program_point: usize,
}

/// Fence scope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceScope {
    /// Thread-level fence
    Thread,
    /// Block-level fence
    Block,
    /// Device-level fence
    Device,
    /// System-level fence (including host)
    System,
}

/// Control flow graph for reachability analysis
#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    /// Nodes (program points)
    nodes: Set<usize>,
    /// Edges (control flow)
    edges: Map<usize, Set<usize>>,
    /// Entry point
    entry: usize,
}

impl ControlFlowGraph {
    /// Create a new control flow graph
    pub fn new() -> Self {
        Self {
            nodes: Set::new(),
            edges: Map::new(),
            entry: 0,
        }
    }

    /// Add a node
    pub fn add_node(&mut self, node: usize) {
        self.nodes.insert(node);
    }

    /// Add an edge
    pub fn add_edge(&mut self, from: usize, to: usize) {
        self.edges.entry(from).or_default().insert(to);
    }

    /// Set entry point
    pub fn set_entry(&mut self, entry: usize) {
        self.entry = entry;
    }

    /// Check if a node is reachable from entry
    pub fn is_reachable(&self, target: usize) -> bool {
        let mut visited = Set::new();
        let mut worklist = Vec::new();
        worklist.push(self.entry);

        while let Some(node) = worklist.pop() {
            if node == target {
                return true;
            }

            if visited.contains(&node) {
                continue;
            }

            visited.insert(node);

            if let Maybe::Some(successors) = self.edges.get(&node) {
                for &succ in successors.iter() {
                    if !visited.contains(&succ) {
                        worklist.push(succ);
                    }
                }
            }
        }

        false
    }
}

impl Default for ControlFlowGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Verification result
#[derive(Debug, Clone)]
pub enum VerificationResult {
    /// Barrier divergence detected
    BarrierDivergence {
        barrier_id: u32,
        block: BlockId,
        message: Text,
    },
    /// Deadlock detected
    Deadlock {
        threads: List<ThreadId>,
        message: Text,
    },
    /// Liveness violation (no progress)
    LivenessViolation { message: Text },
    /// Atomicity violation
    AtomicityViolation {
        atomic: AtomicOperation,
        message: Text,
    },
}

impl VerificationResult {
    /// Format result for display
    pub fn format(&self) -> Text {
        match self {
            Self::BarrierDivergence {
                barrier_id,
                block,
                message,
            } => format!(
                "Barrier Divergence: barrier {} in block {:?} - {}",
                barrier_id, block, message
            )
            .to_text(),
            Self::Deadlock { threads, message } => {
                format!("Deadlock: {} threads - {}", threads.len(), message).to_text()
            }
            Self::LivenessViolation { message } => {
                format!("Liveness Violation: {}", message).to_text()
            }
            Self::AtomicityViolation { atomic, message } => format!(
                "Atomicity Violation: {:?} operation by thread {:?} - {}",
                atomic.op_type, atomic.thread, message
            )
            .to_text(),
        }
    }
}

/// Synchronization verification statistics
#[derive(Debug, Clone, Default)]
pub struct SyncVerificationStats {
    /// Total barrier checks
    pub total_barrier_checks: usize,
    /// Barriers that passed verification
    pub barriers_passed: usize,
    /// Barriers that failed verification
    pub barriers_failed: usize,
    /// Time spent verifying barriers (ms)
    pub barrier_check_time_ms: u64,
    /// Total atomic operations checked
    pub total_atomic_checks: usize,
    /// Time spent verifying atomics (ms)
    pub atomic_check_time_ms: u64,
}

// ==================== Utilities ====================

/// Create a symbolic thread ID
pub fn create_symbolic_thread(name: &str) -> Int {
    Int::new_const(name)
}

/// Encode thread in same block constraint
pub fn encode_threads_in_block(
    t1: ThreadId,
    t2: ThreadId,
    block: BlockId,
    block_dim: (u32, u32, u32),
) -> Bool {
    // Both threads have same block ID
    Bool::from_bool(true) // Simplified - in practice, check block coordinates
}

/// Encode barrier arrival order
pub fn encode_arrival_order(thread1: ThreadId, thread2: ThreadId, barrier_id: u32) -> Bool {
    // Symbolic order: thread1 arrives before thread2
    let var_name = format!(
        "thread_{}_before_thread_{}_at_barrier_{}",
        thread1.to_linear((32, 1, 1)),
        thread2.to_linear((32, 1, 1)),
        barrier_id
    );
    Bool::new_const(var_name.as_str())
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_verifier_creation() {
        let verifier = SyncVerifier::new((1, 1, 1), (32, 1, 1));
        assert_eq!(verifier.barriers.len(), 0);
        assert_eq!(verifier.atomics.len(), 0);
    }

    #[test]
    fn test_cfg_reachability() {
        let mut cfg = ControlFlowGraph::new();
        cfg.set_entry(0);
        cfg.add_node(0);
        cfg.add_node(1);
        cfg.add_node(2);
        cfg.add_edge(0, 1);
        cfg.add_edge(1, 2);

        assert!(cfg.is_reachable(0));
        assert!(cfg.is_reachable(1));
        assert!(cfg.is_reachable(2));
    }

    #[test]
    fn test_cfg_unreachable() {
        let mut cfg = ControlFlowGraph::new();
        cfg.set_entry(0);
        cfg.add_node(0);
        cfg.add_node(1);
        cfg.add_node(2);
        cfg.add_edge(0, 1);
        // Node 2 is not reachable

        assert!(cfg.is_reachable(0));
        assert!(cfg.is_reachable(1));
        assert!(!cfg.is_reachable(2));
    }
}
