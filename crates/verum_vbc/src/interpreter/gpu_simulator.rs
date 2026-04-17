//! GPU CPU-Fallback Semantic Simulator
//!
//! This module provides a CPU-based simulation of GPU kernel execution
//! with correct threading semantics. Each kernel launch iterates over
//! all blocks in the grid and all threads within each block, providing
//! each thread with its own identity (threadIdx, blockIdx) and access
//! to per-block shared memory.
//!
//! ## Threading Model
//!
//! Threads within a block execute sequentially in the CPU simulation.
//! This is semantically correct for well-synchronized GPU programs
//! because `__syncthreads()` is a no-op when threads execute in order.
//!
//! Atomic operations are regular operations in single-threaded execution,
//! which is also correct (no data races in sequential execution).
//!
//! ## Shared Memory
//!
//! Each block gets its own shared memory buffer that persists for the
//! duration of the block's execution. All threads within a block can
//! read and write to this shared memory.

use std::collections::HashMap;

/// GPU thread context providing identity and shared resources.
///
/// This is set before each kernel thread execution and can be queried
/// via GPU thread intrinsic sub-opcodes (0xA0-0xAF).
#[derive(Debug, Clone)]
pub struct GpuThreadContext {
    /// Thread index within the current block (x, y, z).
    pub thread_id: [u32; 3],
    /// Block index within the grid (x, y, z).
    pub block_id: [u32; 3],
    /// Block dimensions (number of threads per block in each dimension).
    pub block_dim: [u32; 3],
    /// Grid dimensions (number of blocks in each dimension).
    pub grid_dim: [u32; 3],
    /// Per-block shared memory buffer.
    /// Indexed by byte offset.
    pub shared_memory: Vec<u8>,
    /// Shared memory size in bytes.
    pub shared_mem_size: usize,
    /// Warp size (32 for NVIDIA, 64 for AMD, 32 for CPU simulation).
    pub warp_size: u32,
}

impl GpuThreadContext {
    /// Create a new GPU thread context for the given position in the grid.
    pub fn new(
        thread_id: [u32; 3],
        block_id: [u32; 3],
        block_dim: [u32; 3],
        grid_dim: [u32; 3],
        shared_memory: &SharedMemoryBlock,
    ) -> Self {
        Self {
            thread_id,
            block_id,
            block_dim,
            grid_dim,
            shared_memory: Vec::new(), // Shared memory is owned by the block, not the thread
            shared_mem_size: shared_memory.size,
            warp_size: 32,
        }
    }

    /// Linear thread index within the block.
    pub fn linear_thread_id(&self) -> u32 {
        self.thread_id[0]
            + self.thread_id[1] * self.block_dim[0]
            + self.thread_id[2] * self.block_dim[0] * self.block_dim[1]
    }

    /// Linear block index within the grid.
    pub fn linear_block_id(&self) -> u32 {
        self.block_id[0]
            + self.block_id[1] * self.grid_dim[0]
            + self.block_id[2] * self.grid_dim[0] * self.grid_dim[1]
    }

    /// Total number of threads in a block.
    pub fn threads_per_block(&self) -> u32 {
        self.block_dim[0] * self.block_dim[1] * self.block_dim[2]
    }

    /// Total number of blocks in the grid.
    pub fn total_blocks(&self) -> u32 {
        self.grid_dim[0] * self.grid_dim[1] * self.grid_dim[2]
    }

    /// Warp index of the current thread.
    pub fn warp_id(&self) -> u32 {
        self.linear_thread_id() / self.warp_size
    }

    /// Lane index within the warp.
    pub fn lane_id(&self) -> u32 {
        self.linear_thread_id() % self.warp_size
    }
}

/// Per-block shared memory allocation.
///
/// Each block in the grid gets its own shared memory buffer that persists
/// for the duration of the block's execution across all threads.
#[derive(Debug)]
pub struct SharedMemoryBlock {
    /// Shared memory buffer.
    pub data: Vec<u8>,
    /// Size in bytes.
    pub size: usize,
}

impl SharedMemoryBlock {
    /// Create a new shared memory block of the given size, zero-initialized.
    pub fn new(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
            size,
        }
    }

    /// Read a value from shared memory at the given byte offset.
    pub fn read_i64(&self, offset: usize) -> Option<i64> {
        if offset + 8 > self.size {
            return None;
        }
        let bytes: [u8; 8] = self.data[offset..offset + 8].try_into().ok()?;
        Some(i64::from_le_bytes(bytes))
    }

    /// Write a value to shared memory at the given byte offset.
    pub fn write_i64(&mut self, offset: usize, value: i64) -> bool {
        if offset + 8 > self.size {
            return false;
        }
        self.data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        true
    }

    /// Read a f64 from shared memory at the given byte offset.
    pub fn read_f64(&self, offset: usize) -> Option<f64> {
        if offset + 8 > self.size {
            return None;
        }
        let bytes: [u8; 8] = self.data[offset..offset + 8].try_into().ok()?;
        Some(f64::from_le_bytes(bytes))
    }

    /// Write a f64 to shared memory at the given byte offset.
    pub fn write_f64(&mut self, offset: usize, value: f64) -> bool {
        if offset + 8 > self.size {
            return false;
        }
        self.data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        true
    }

    /// Read a u32 from shared memory at the given byte offset.
    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        if offset + 4 > self.size {
            return None;
        }
        let bytes: [u8; 4] = self.data[offset..offset + 4].try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    /// Write a u32 to shared memory at the given byte offset.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> bool {
        if offset + 4 > self.size {
            return false;
        }
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        true
    }

    /// Atomic add on a i64 value at the given byte offset.
    /// Returns the previous value.
    /// In sequential execution, this is just read-modify-write.
    pub fn atomic_add_i64(&mut self, offset: usize, value: i64) -> Option<i64> {
        let old = self.read_i64(offset)?;
        self.write_i64(offset, old + value);
        Some(old)
    }

    /// Atomic add on a f64 value at the given byte offset.
    /// Returns the previous value.
    pub fn atomic_add_f64(&mut self, offset: usize, value: f64) -> Option<f64> {
        let old = self.read_f64(offset)?;
        self.write_f64(offset, old + value);
        Some(old)
    }

    /// Atomic compare-and-swap on a i64 value at the given byte offset.
    /// Returns the previous value.
    pub fn atomic_cas_i64(&mut self, offset: usize, expected: i64, desired: i64) -> Option<i64> {
        let old = self.read_i64(offset)?;
        if old == expected {
            self.write_i64(offset, desired);
        }
        Some(old)
    }

    /// Atomic max on a i64 value.
    /// Returns the previous value.
    pub fn atomic_max_i64(&mut self, offset: usize, value: i64) -> Option<i64> {
        let old = self.read_i64(offset)?;
        if value > old {
            self.write_i64(offset, value);
        }
        Some(old)
    }

    /// Atomic min on a i64 value.
    /// Returns the previous value.
    pub fn atomic_min_i64(&mut self, offset: usize, value: i64) -> Option<i64> {
        let old = self.read_i64(offset)?;
        if value < old {
            self.write_i64(offset, value);
        }
        Some(old)
    }

    /// Clear shared memory to zero.
    pub fn clear(&mut self) {
        self.data.fill(0);
    }
}

/// Parameters for a GPU kernel launch.
///
/// Saved from the Launch instruction bytecode encoding and used
/// to drive the CPU-fallback grid/block iteration.
#[derive(Debug, Clone)]
pub struct KernelLaunchParams {
    /// Kernel function ID.
    pub kernel_id: u32,
    /// Grid dimensions (blocks in each dimension).
    pub grid_dim: [u32; 3],
    /// Block dimensions (threads per block in each dimension).
    pub block_dim: [u32; 3],
    /// Shared memory size in bytes per block.
    pub shared_mem_size: usize,
    /// Argument values for the kernel function.
    pub args: Vec<i64>,
}

impl KernelLaunchParams {
    /// Total number of threads across all blocks.
    pub fn total_threads(&self) -> u64 {
        let blocks = self.grid_dim[0] as u64 * self.grid_dim[1] as u64 * self.grid_dim[2] as u64;
        let threads = self.block_dim[0] as u64 * self.block_dim[1] as u64 * self.block_dim[2] as u64;
        blocks * threads
    }

    /// Returns an iterator over all (block_id, thread_id) pairs.
    pub fn thread_iter(&self) -> ThreadIterator {
        ThreadIterator {
            grid_dim: self.grid_dim,
            block_dim: self.block_dim,
            current_block: [0, 0, 0],
            current_thread: [0, 0, 0],
            done: false,
        }
    }
}

/// Iterator over all (block_id, thread_id) pairs in a kernel launch.
///
/// Iterates blocks in x,y,z order (x varies fastest), and within each
/// block iterates threads in x,y,z order (matching CUDA semantics).
pub struct ThreadIterator {
    grid_dim: [u32; 3],
    block_dim: [u32; 3],
    current_block: [u32; 3],
    current_thread: [u32; 3],
    done: bool,
}

impl Iterator for ThreadIterator {
    type Item = ([u32; 3], [u32; 3]); // (block_id, thread_id)

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        let result = (self.current_block, self.current_thread);

        // Advance thread_id
        self.current_thread[0] += 1;
        if self.current_thread[0] >= self.block_dim[0] {
            self.current_thread[0] = 0;
            self.current_thread[1] += 1;
            if self.current_thread[1] >= self.block_dim[1] {
                self.current_thread[1] = 0;
                self.current_thread[2] += 1;
                if self.current_thread[2] >= self.block_dim[2] {
                    // Block complete, advance to next block
                    self.current_thread = [0, 0, 0];
                    self.current_block[0] += 1;
                    if self.current_block[0] >= self.grid_dim[0] {
                        self.current_block[0] = 0;
                        self.current_block[1] += 1;
                        if self.current_block[1] >= self.grid_dim[1] {
                            self.current_block[1] = 0;
                            self.current_block[2] += 1;
                            if self.current_block[2] >= self.grid_dim[2] {
                                self.done = true;
                            }
                        }
                    }
                }
            }
        }

        Some(result)
    }
}

/// Tracks global memory for GPU atomics across the entire kernel execution.
///
/// In CPU fallback mode, "global" memory is just regular host memory.
/// Atomic operations on global memory addresses use this tracker to
/// ensure correct sequential semantics.
#[derive(Debug, Default)]
pub struct GpuGlobalMemory {
    /// Atomic values tracked by address.
    pub atomics: HashMap<usize, i64>,
}

impl GpuGlobalMemory {
    /// Create a new empty GPU global memory.
    pub fn new() -> Self {
        Self {
            atomics: HashMap::new(),
        }
    }

    /// Atomic add on a global memory address.
    pub fn atomic_add(&mut self, addr: usize, value: i64) -> i64 {
        let entry = self.atomics.entry(addr).or_insert(0);
        let old = *entry;
        *entry += value;
        old
    }

    /// Atomic CAS on a global memory address.
    pub fn atomic_cas(&mut self, addr: usize, expected: i64, desired: i64) -> i64 {
        let entry = self.atomics.entry(addr).or_insert(0);
        let old = *entry;
        if old == expected {
            *entry = desired;
        }
        old
    }

    /// Atomic max on a global memory address.
    pub fn atomic_max(&mut self, addr: usize, value: i64) -> i64 {
        let entry = self.atomics.entry(addr).or_insert(i64::MIN);
        let old = *entry;
        if value > old {
            *entry = value;
        }
        old
    }

    /// Atomic min on a global memory address.
    pub fn atomic_min(&mut self, addr: usize, value: i64) -> i64 {
        let entry = self.atomics.entry(addr).or_insert(i64::MAX);
        let old = *entry;
        if value < old {
            *entry = value;
        }
        old
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_iterator_1d() {
        let params = KernelLaunchParams {
            kernel_id: 0,
            grid_dim: [2, 1, 1],
            block_dim: [3, 1, 1],
            shared_mem_size: 0,
            args: vec![],
        };

        let threads: Vec<_> = params.thread_iter().collect();
        assert_eq!(threads.len(), 6); // 2 blocks * 3 threads
        // Block 0: threads 0,1,2
        assert_eq!(threads[0], ([0, 0, 0], [0, 0, 0]));
        assert_eq!(threads[1], ([0, 0, 0], [1, 0, 0]));
        assert_eq!(threads[2], ([0, 0, 0], [2, 0, 0]));
        // Block 1: threads 0,1,2
        assert_eq!(threads[3], ([1, 0, 0], [0, 0, 0]));
        assert_eq!(threads[4], ([1, 0, 0], [1, 0, 0]));
        assert_eq!(threads[5], ([1, 0, 0], [2, 0, 0]));
    }

    #[test]
    fn test_thread_iterator_2d() {
        let params = KernelLaunchParams {
            kernel_id: 0,
            grid_dim: [1, 1, 1],
            block_dim: [2, 2, 1],
            shared_mem_size: 0,
            args: vec![],
        };

        let threads: Vec<_> = params.thread_iter().collect();
        assert_eq!(threads.len(), 4);
        assert_eq!(threads[0], ([0, 0, 0], [0, 0, 0]));
        assert_eq!(threads[1], ([0, 0, 0], [1, 0, 0]));
        assert_eq!(threads[2], ([0, 0, 0], [0, 1, 0]));
        assert_eq!(threads[3], ([0, 0, 0], [1, 1, 0]));
    }

    #[test]
    fn test_shared_memory_read_write() {
        let mut smem = SharedMemoryBlock::new(64);
        assert!(smem.write_i64(0, 42));
        assert_eq!(smem.read_i64(0), Some(42));

        assert!(smem.write_f64(8, 3.14));
        assert_eq!(smem.read_f64(8), Some(3.14));

        // Out of bounds
        assert!(!smem.write_i64(60, 0));
        assert_eq!(smem.read_i64(60), None);
    }

    #[test]
    fn test_shared_memory_atomics() {
        let mut smem = SharedMemoryBlock::new(64);
        smem.write_i64(0, 10);

        let old = smem.atomic_add_i64(0, 5).unwrap();
        assert_eq!(old, 10);
        assert_eq!(smem.read_i64(0), Some(15));

        let old = smem.atomic_cas_i64(0, 15, 100).unwrap();
        assert_eq!(old, 15);
        assert_eq!(smem.read_i64(0), Some(100));

        // CAS fails when expected doesn't match
        let old = smem.atomic_cas_i64(0, 15, 200).unwrap();
        assert_eq!(old, 100);
        assert_eq!(smem.read_i64(0), Some(100)); // Unchanged
    }

    #[test]
    fn test_gpu_thread_context() {
        let smem = SharedMemoryBlock::new(256);
        let ctx = GpuThreadContext::new(
            [5, 2, 0],
            [1, 3, 0],
            [8, 4, 1],
            [4, 4, 1],
            &smem,
        );

        assert_eq!(ctx.thread_id, [5, 2, 0]);
        assert_eq!(ctx.block_id, [1, 3, 0]);
        assert_eq!(ctx.linear_thread_id(), 5 + 2 * 8); // 21
        assert_eq!(ctx.linear_block_id(), 1 + 3 * 4); // 13
        assert_eq!(ctx.threads_per_block(), 32);
        assert_eq!(ctx.total_blocks(), 16);
        assert_eq!(ctx.warp_id(), 21 / 32); // 0
        assert_eq!(ctx.lane_id(), 21); // 21
    }

    #[test]
    fn test_global_memory_atomics() {
        let mut gmem = GpuGlobalMemory::new();

        let old = gmem.atomic_add(0x1000, 10);
        assert_eq!(old, 0);

        let old = gmem.atomic_add(0x1000, 5);
        assert_eq!(old, 10);
        assert_eq!(*gmem.atomics.get(&0x1000).unwrap(), 15);
    }

    #[test]
    fn test_total_threads() {
        let params = KernelLaunchParams {
            kernel_id: 0,
            grid_dim: [4, 4, 1],
            block_dim: [256, 1, 1],
            shared_mem_size: 0,
            args: vec![],
        };
        assert_eq!(params.total_threads(), 4096);
    }
}
