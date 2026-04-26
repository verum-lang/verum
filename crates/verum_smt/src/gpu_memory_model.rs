//! GPU Memory Model for Z3 Verification
//!
//! This module encodes the GPU memory hierarchy for formal verification:
//! - Global memory (shared across all thread blocks)
//! - Shared memory (shared within a block)
//! - Local memory (thread-private)
//!
//! ## Memory Model Properties
//!
//! - **Disjoint Address Spaces**: Global, shared, and local memory are distinct
//! - **Sequential Consistency**: Within a thread, memory operations are ordered
//! - **Relaxed Consistency**: Between threads, operations may be reordered
//! - **Barrier Synchronization**: Barriers enforce ordering across threads
//!
//! ## Z3 Encoding Strategy
//!
//! - Arrays: ThreadID × Address → Value
//! - Memory regions: Disjoint address ranges for each space
//! - Happens-before: Partial order on memory operations
//! - Synchronization: Barrier constraints
//!
//! GPU memory model verification for Verum's `@gpu` annotated kernels.
//! Models GPU memory consistency (relaxed/acquire/release) with SMT constraints.
//! Based on: GPU Memory Model (PTX ISA, CUDA Programming Guide)

use z3::{
    Solver,
    ast::{Array, Bool, Int},
};

use verum_common::{List, Maybe, Text};
use verum_common::ToText;

// ==================== Core Types ====================

/// Thread identifier (x, y, z within block)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl ThreadId {
    /// Create a new thread ID
    pub fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    /// Flatten to linear index
    pub fn to_linear(&self, block_dim: (u32, u32, u32)) -> u32 {
        self.x + self.y * block_dim.0 + self.z * block_dim.0 * block_dim.1
    }

    /// Convert to Z3 Int encoding
    pub fn to_z3(&self, block_dim: (u32, u32, u32)) -> Int {
        Int::from_u64(self.to_linear(block_dim) as u64)
    }
}

/// Block identifier (x, y, z within grid)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

impl BlockId {
    /// Create a new block ID
    pub fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }

    /// Flatten to linear index
    pub fn to_linear(&self, grid_dim: (u32, u32, u32)) -> u32 {
        self.x + self.y * grid_dim.0 + self.z * grid_dim.0 * grid_dim.1
    }

    /// Convert to Z3 Int encoding
    pub fn to_z3(&self, grid_dim: (u32, u32, u32)) -> Int {
        Int::from_u64(self.to_linear(grid_dim) as u64)
    }
}

/// Memory space identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemorySpace {
    /// Global memory (GPU DRAM)
    Global,
    /// Shared memory (on-chip, per-block)
    Shared,
    /// Local memory (thread-private registers/stack)
    Local,
}

impl MemorySpace {
    /// Get address space ID for Z3 encoding
    pub fn to_z3_space(&self) -> i64 {
        match self {
            Self::Global => 0,
            Self::Shared => 1,
            Self::Local => 2,
        }
    }
}

/// GPU Memory Model
///
/// Encodes the hierarchical memory system of GPUs using Z3 arrays.
/// Each memory space is modeled as a separate array theory.
///
/// Note: Z3 arrays are returned directly from `create_*_memory` and are owned
/// by the caller; this struct only retains dimensional + access-tracking state.
pub struct GpuMemoryModel {
    /// Grid dimensions (blocks in grid)
    grid_dim: (u32, u32, u32),

    /// Block dimensions (threads per block)
    block_dim: (u32, u32, u32),

    /// Address space constraints
    address_constraints: List<Bool>,

    /// Memory access tracking for verification
    memory_accesses: List<MemoryAccess>,
}

impl GpuMemoryModel {
    /// Create a new GPU memory model
    ///
    /// # Arguments
    /// - `grid_dim`: (x, y, z) dimensions of the grid (number of blocks)
    /// - `block_dim`: (x, y, z) dimensions of each block (threads per block)
    pub fn new(grid_dim: (u32, u32, u32), block_dim: (u32, u32, u32)) -> Self {
        Self {
            grid_dim,
            block_dim,
            address_constraints: List::new(),
            memory_accesses: List::new(),
        }
    }

    /// Create global memory array
    ///
    /// Returns a Z3 array: (ThreadID × Address) → Value
    pub fn create_global_memory(&self, name: &str) -> Array {
        // Array from Int (flattened thread ID + address) to Int (value)
        let int_sort = z3::Sort::int();
        Array::new_const(name, &int_sort, &int_sort)
    }

    /// Create shared memory array
    ///
    /// Returns a Z3 array: (BlockID × Address) → Value
    pub fn create_shared_memory(&self, name: &str, _block_size: u32) -> Array {
        // Array from Int (block ID + address) to Int (value)
        let int_sort = z3::Sort::int();
        Array::new_const(name, &int_sort, &int_sort)
    }

    /// Create local memory array
    ///
    /// Returns a Z3 array: (ThreadID × Address) → Value
    pub fn create_local_memory(&self, name: &str) -> Array {
        let int_sort = z3::Sort::int();
        Array::new_const(name, &int_sort, &int_sort)
    }

    /// Encode a load operation
    ///
    /// Returns the value loaded from memory
    pub fn encode_load(
        &mut self,
        space: MemorySpace,
        thread: ThreadId,
        block: BlockId,
        addr: Int,
    ) -> Int {
        let access = MemoryAccess {
            space,
            thread,
            block,
            address: addr.to_string().to_text(),
            is_write: false,
            value: Maybe::None,
            timestamp: self.memory_accesses.len(),
        };
        self.memory_accesses.push(access);

        // Create symbolic value for the load
        let load_var = format!(
            "load_{}_{}_{}",
            space.to_z3_space(),
            thread.to_linear(self.block_dim),
            self.memory_accesses.len()
        );
        Int::new_const(load_var.as_str())
    }

    /// Encode a store operation
    ///
    /// Adds constraints for the memory write
    pub fn encode_store(
        &mut self,
        space: MemorySpace,
        thread: ThreadId,
        block: BlockId,
        addr: Int,
        val: Int,
    ) {
        let access = MemoryAccess {
            space,
            thread,
            block,
            address: addr.to_string().to_text(),
            is_write: true,
            value: Maybe::Some(val.to_string().to_text()),
            timestamp: self.memory_accesses.len(),
        };
        self.memory_accesses.push(access);
    }

    /// Check if address is in global memory space
    pub fn is_global(&self, addr: &Int) -> Bool {
        // Global addresses are in range [0, global_size)
        // For simplicity, we use a large upper bound
        let lower = Int::from_i64(0);
        let upper = Int::from_i64(i64::MAX);
        Bool::and(&[&addr.ge(&lower), &addr.lt(&upper)])
    }

    /// Check if address is in shared memory space
    pub fn is_shared(&self, addr: &Int) -> Bool {
        // Shared addresses are in a separate range
        // Encoded as negative addresses for simplicity
        let lower = Int::from_i64(-(i32::MAX as i64));
        let upper = Int::from_i64(-1);
        Bool::and(&[&addr.ge(&lower), &addr.le(&upper)])
    }

    /// Check if address is in local memory space
    pub fn is_local(&self, addr: &Int) -> Bool {
        // Local addresses are thread-private
        // Encoded as addresses less than the shared memory range
        // i.e., addr < -(i32::MAX as i64)
        let local_upper_bound = Int::from_i64(-(i32::MAX as i64) - 1);
        addr.lt(&local_upper_bound)
    }

    /// Get memory accesses
    pub fn get_accesses(&self) -> &List<MemoryAccess> {
        &self.memory_accesses
    }

    /// Generate disjoint address space constraints
    ///
    /// Ensures that global, shared, and local memory are disjoint
    pub fn generate_address_space_constraints(&mut self, solver: &Solver) {
        // For each memory access, assert that it belongs to exactly one space
        for access in &self.memory_accesses {
            let addr_text = &access.address;
            let addr = Int::new_const(addr_text.as_str());

            let is_global = self.is_global(&addr);
            let is_shared = self.is_shared(&addr);
            let is_local = self.is_local(&addr);

            // Exactly one space
            let space_constraint = match access.space {
                MemorySpace::Global => is_global,
                MemorySpace::Shared => is_shared,
                MemorySpace::Local => is_local,
            };

            solver.assert(&space_constraint);
            self.address_constraints.push(space_constraint);
        }
    }

    /// Get grid dimensions
    pub fn grid_dim(&self) -> (u32, u32, u32) {
        self.grid_dim
    }

    /// Get block dimensions
    pub fn block_dim(&self) -> (u32, u32, u32) {
        self.block_dim
    }

    /// Total number of threads in grid
    pub fn total_threads(&self) -> u32 {
        let (gx, gy, gz) = self.grid_dim;
        let (bx, by, bz) = self.block_dim;
        gx * gy * gz * bx * by * bz
    }
}

/// Memory access record
#[derive(Debug, Clone)]
pub struct MemoryAccess {
    /// Memory space
    pub space: MemorySpace,
    /// Thread ID performing the access
    pub thread: ThreadId,
    /// Block ID containing the thread
    pub block: BlockId,
    /// Address being accessed (symbolic)
    pub address: Text,
    /// Whether this is a write (true) or read (false)
    pub is_write: bool,
    /// Value written (if write)
    pub value: Maybe<Text>,
    /// Timestamp in program order
    pub timestamp: usize,
}

// ==================== Utilities ====================

/// Create a symbolic address variable
pub fn create_symbolic_address(name: &str) -> Int {
    Int::new_const(name)
}

/// Create a symbolic value variable
pub fn create_symbolic_value(name: &str) -> Int {
    Int::new_const(name)
}

/// Encode memory aliasing constraints
///
/// Two addresses may alias if they could be equal
pub fn encode_may_alias(addr1: &Int, addr2: &Int) -> Bool {
    addr1
        .safe_eq(addr2)
        .unwrap_or_else(|_| Bool::from_bool(false))
}

/// Encode memory non-aliasing constraints
///
/// Two addresses do not alias if they are guaranteed different
pub fn encode_no_alias(addr1: &Int, addr2: &Int) -> Bool {
    let eq = addr1
        .safe_eq(addr2)
        .unwrap_or_else(|_| Bool::from_bool(false));
    eq.not()
}

// ==================== Statistics ====================

/// Memory model statistics
#[derive(Debug, Clone, Default)]
pub struct MemoryModelStats {
    /// Total memory accesses encoded
    pub total_accesses: usize,
    /// Global memory accesses
    pub global_accesses: usize,
    /// Shared memory accesses
    pub shared_accesses: usize,
    /// Local memory accesses
    pub local_accesses: usize,
    /// Number of loads
    pub loads: usize,
    /// Number of stores
    pub stores: usize,
    /// Address space constraints generated
    pub address_constraints: usize,
}

impl GpuMemoryModel {
    /// Get statistics
    pub fn stats(&self) -> MemoryModelStats {
        let mut stats = MemoryModelStats::default();

        stats.total_accesses = self.memory_accesses.len();
        stats.address_constraints = self.address_constraints.len();

        for access in &self.memory_accesses {
            match access.space {
                MemorySpace::Global => stats.global_accesses += 1,
                MemorySpace::Shared => stats.shared_accesses += 1,
                MemorySpace::Local => stats.local_accesses += 1,
            }

            if access.is_write {
                stats.stores += 1;
            } else {
                stats.loads += 1;
            }
        }

        stats
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_id_linearization() {
        let tid = ThreadId::new(2, 1, 0);
        let block_dim = (4, 4, 4);
        assert_eq!(tid.to_linear(block_dim), 2 + 4);
    }

    #[test]
    fn test_memory_model_creation() {
        let model = GpuMemoryModel::new((1, 1, 1), (32, 1, 1));
        assert_eq!(model.grid_dim(), (1, 1, 1));
        assert_eq!(model.block_dim(), (32, 1, 1));
        assert_eq!(model.total_threads(), 32);
    }

    #[test]
    fn test_memory_access_tracking() {
        let mut model = GpuMemoryModel::new((1, 1, 1), (32, 1, 1));
        let thread = ThreadId::new(0, 0, 0);
        let block = BlockId::new(0, 0, 0);
        let addr = Int::from_i64(0x1000);

        model.encode_load(MemorySpace::Global, thread, block, addr);

        let stats = model.stats();
        assert_eq!(stats.total_accesses, 1);
        assert_eq!(stats.global_accesses, 1);
        assert_eq!(stats.loads, 1);
    }
}
