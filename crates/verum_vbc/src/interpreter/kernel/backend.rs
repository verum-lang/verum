//! Backend trait for device-agnostic tensor operations.
//!
//! This module defines the `Backend` trait that abstracts compute backends
//! (CPU, CUDA, Metal, etc.) for tensor operations.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::device::DeviceId;

// ============================================================================
// Send-safe pointer wrapper
// ============================================================================

/// A wrapper around NonNull<u8> that is Send + Sync.
///
/// # Safety
/// The user must ensure that the pointer is valid for the lifetime of this wrapper
/// and that no data races occur when accessing the pointed-to data.
#[derive(Debug, Clone, Copy)]
pub struct SendPtr(pub *mut u8);

unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

impl SendPtr {
    /// Create from NonNull
    pub fn from_non_null(ptr: NonNull<u8>) -> Self {
        Self(ptr.as_ptr())
    }

    /// Convert to NonNull
    pub fn to_non_null(self) -> Option<NonNull<u8>> {
        NonNull::new(self.0)
    }
}
use super::super::tensor::TensorHandle;
use crate::instruction::{TensorBinaryOp, TensorReduceOp, TensorUnaryOp};

// ============================================================================
// Backend Trait
// ============================================================================

/// Backend abstraction for device-specific operations.
///
/// Implements compute operations for a specific device type (CPU, GPU).
/// Each backend handles memory allocation, data transfer, and compute kernels.
pub trait Backend: Send + Sync {
    /// Backend name for diagnostics
    fn name(&self) -> &'static str;

    /// Device ID this backend handles
    fn device_id(&self) -> DeviceId;

    /// Get backend capabilities
    fn capabilities(&self) -> &ComputeCapabilities;

    // Memory operations
    /// Allocate device memory
    fn allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>>;

    /// Deallocate device memory
    fn deallocate(&self, ptr: NonNull<u8>, size: usize, align: usize);

    /// Copy host to device
    fn copy_h2d(&self, host: *const u8, device: NonNull<u8>, size: usize);

    /// Copy device to host
    fn copy_d2h(&self, device: NonNull<u8>, host: *mut u8, size: usize);

    /// Copy device to device (same device)
    fn copy_d2d(&self, src: NonNull<u8>, dst: NonNull<u8>, size: usize);

    /// Synchronize device (wait for all operations to complete)
    fn synchronize(&self);

    // Compute operations
    /// Binary operation (element-wise)
    fn binop(
        &self,
        a: &TensorHandle,
        b: &TensorHandle,
        op: TensorBinaryOp,
    ) -> Option<TensorHandle>;

    /// Unary operation (element-wise)
    fn unop(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle>;

    /// Reduction operation
    fn reduce(
        &self,
        a: &TensorHandle,
        op: TensorReduceOp,
        axis: Option<usize>,
    ) -> Option<TensorHandle>;

    /// Matrix multiplication
    fn matmul(&self, a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle>;

    /// Memory info (free, total)
    fn memory_info(&self) -> (usize, usize);
}

/// Backend capabilities for dispatch decisions
#[derive(Debug, Clone)]
pub struct ComputeCapabilities {
    /// Maximum number of threads
    pub max_threads: usize,
    /// SIMD width in elements (1, 4, 8, 16)
    pub simd_width: usize,
    /// Has FMA support
    pub has_fma: bool,
    /// Has tensor cores (GPU)
    pub has_tensor_cores: bool,
    /// Maximum shared memory (GPU)
    pub max_shared_memory: usize,
    /// Compute capability (major, minor) for CUDA
    pub compute_capability: (u32, u32),
    /// Memory bandwidth in GB/s
    pub memory_bandwidth_gbps: f64,
    /// Peak TFLOPS for F32
    pub peak_tflops_f32: f64,
}

impl Default for ComputeCapabilities {
    fn default() -> Self {
        Self {
            max_threads: 1,
            simd_width: 1,
            has_fma: false,
            has_tensor_cores: false,
            max_shared_memory: 0,
            compute_capability: (0, 0),
            memory_bandwidth_gbps: 0.0,
            peak_tflops_f32: 0.0,
        }
    }
}

// ============================================================================
// Memory Pool
// ============================================================================

/// Power-of-two memory pool for reduced fragmentation.
///
/// Maintains free lists for different allocation sizes, allowing
/// quick reuse of recently freed allocations.
pub struct MemoryPool {
    /// Device this pool manages
    device: DeviceId,
    /// Free lists for power-of-two sizes (2^0 to 2^31)
    /// Each bucket contains pointers to free blocks of that size
    free_lists: [std::sync::Mutex<Vec<SendPtr>>; 32],
    /// Total bytes currently allocated (not including pooled)
    allocated: AtomicUsize,
    /// Peak allocation
    peak: AtomicUsize,
    /// Cache hits (reused from pool)
    cache_hits: AtomicUsize,
    /// Cache misses (new allocation)
    cache_misses: AtomicUsize,
}

impl MemoryPool {
    /// Create a new memory pool for a device
    pub fn new(device: DeviceId) -> Self {
        Self {
            device,
            free_lists: std::array::from_fn(|_| std::sync::Mutex::new(Vec::new())),
            allocated: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            cache_hits: AtomicUsize::new(0),
            cache_misses: AtomicUsize::new(0),
        }
    }

    /// Allocate from pool, using cached allocation if available
    pub fn allocate<B: Backend>(&self, size: usize, backend: &B) -> Option<NonNull<u8>> {
        if size == 0 {
            return None;
        }

        let bucket = size.next_power_of_two().trailing_zeros() as usize;
        if bucket >= 32 {
            return None;
        }

        // Try to reuse cached allocation. Recover from poisoning — the
        // free list is a Vec<SendPtr>, which at worst contains a stale
        // pointer if the previous lock holder panicked mid-update. That's
        // safe to observe; we just fall through to allocating fresh.
        {
            let mut list = self.free_lists[bucket]
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(send_ptr) = list.pop() {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                return send_ptr.to_non_null();
            }
        }

        // Allocate new
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
        let alloc_size = 1 << bucket;
        let ptr = backend.allocate(alloc_size, 64)?;

        // Update stats
        let new_allocated = self.allocated.fetch_add(alloc_size, Ordering::Relaxed) + alloc_size;
        self.peak.fetch_max(new_allocated, Ordering::Relaxed);

        Some(ptr)
    }

    /// Return allocation to pool
    pub fn deallocate(&self, ptr: NonNull<u8>, size: usize) {
        if size == 0 {
            return;
        }

        let bucket = size.next_power_of_two().trailing_zeros() as usize;
        if bucket >= 32 {
            return;
        }

        // Never silently drop a deallocation on poison — recover the lock
        // and record the freed pointer so we don't leak on pathological
        // panic-during-alloc paths.
        let mut list = self.free_lists[bucket]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        list.push(SendPtr::from_non_null(ptr));
    }

    /// Release all cached allocations
    pub fn trim<B: Backend>(&self, backend: &B) {
        for (bucket, list_mutex) in self.free_lists.iter().enumerate() {
            // Recover from poisoning — trim is best-effort cleanup.
            let mut list = list_mutex
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let size = 1usize << bucket;
            while let Some(send_ptr) = list.pop() {
                if let Some(ptr) = send_ptr.to_non_null() {
                    backend.deallocate(ptr, size, 64);
                    self.allocated.fetch_sub(size, Ordering::Relaxed);
                }
            }
        }
    }

    /// Get statistics
    pub fn stats(&self) -> MemoryPoolStats {
        MemoryPoolStats {
            device: self.device,
            allocated: self.allocated.load(Ordering::Relaxed),
            peak: self.peak.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
        }
    }
}

/// Memory pool statistics
#[derive(Debug, Clone)]
pub struct MemoryPoolStats {
    /// Device ID
    pub device: DeviceId,
    /// Current allocation (bytes)
    pub allocated: usize,
    /// Peak allocation (bytes)
    pub peak: usize,
    /// Cache hits
    pub cache_hits: usize,
    /// Cache misses
    pub cache_misses: usize,
}

impl MemoryPoolStats {
    /// Cache hit rate (0.0 - 1.0)
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

// ============================================================================
// Sync Flags for Host/Device Coherence
// ============================================================================

bitflags::bitflags! {
    /// Synchronization flags for host/device memory coherence
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SyncFlags: u8 {
        /// Host data modified, device copy stale
        const HOST_DIRTY = 0x01;
        /// Device data modified, host copy stale
        const DEVICE_DIRTY = 0x02;
        /// Both copies in sync
        const SYNCED = 0x00;
    }
}

// ============================================================================
// CPU Backend Implementation
// ============================================================================

/// CPU backend implementation
pub struct CpuBackend {
    capabilities: ComputeCapabilities,
}

impl CpuBackend {
    /// Create new CPU backend with detected capabilities
    #[allow(clippy::field_reassign_with_default)] // Complex conditional initialization
    pub fn new() -> Self {
        let mut caps = ComputeCapabilities::default();

        caps.max_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        #[cfg(target_arch = "x86_64")]
        {
            if std::arch::is_x86_feature_detected!("avx512f") {
                caps.simd_width = 16;
            } else if std::arch::is_x86_feature_detected!("avx2") {
                caps.simd_width = 8;
            } else if std::arch::is_x86_feature_detected!("sse4.2") {
                caps.simd_width = 4;
            }
            if std::arch::is_x86_feature_detected!("fma") {
                caps.has_fma = true;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            caps.simd_width = 4; // NEON
        }

        // Estimate CPU performance (very rough)
        caps.peak_tflops_f32 = (caps.max_threads * caps.simd_width * 2) as f64 * 3.0 / 1000.0;
        caps.memory_bandwidth_gbps = 50.0; // Typical DDR4

        Self { capabilities: caps }
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CpuBackend {
    fn name(&self) -> &'static str {
        "CPU"
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::CPU
    }

    fn capabilities(&self) -> &ComputeCapabilities {
        &self.capabilities
    }

    fn allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        use std::alloc::{alloc, Layout};
        let layout = Layout::from_size_align(size, align).ok()?;
        let ptr = unsafe { alloc(layout) };
        NonNull::new(ptr)
    }

    fn deallocate(&self, ptr: NonNull<u8>, size: usize, align: usize) {
        use std::alloc::{dealloc, Layout};
        if let Ok(layout) = Layout::from_size_align(size, align) {
            unsafe { dealloc(ptr.as_ptr(), layout) };
        }
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)] // Trait-required signature with raw pointers
    fn copy_h2d(&self, host: *const u8, device: NonNull<u8>, size: usize) {
        assert!(!host.is_null(), "copy_h2d: host pointer is null");
        // CPU: host and device are the same address space
        unsafe {
            // SAFETY: host is non-null (asserted), device is NonNull, caller guarantees
            // both buffers are at least `size` bytes and do not overlap.
            std::ptr::copy_nonoverlapping(host, device.as_ptr(), size);
        }
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)] // Trait-required signature with raw pointers
    fn copy_d2h(&self, device: NonNull<u8>, host: *mut u8, size: usize) {
        assert!(!host.is_null(), "copy_d2h: host pointer is null");
        unsafe {
            // SAFETY: device is NonNull, host is non-null (asserted), caller guarantees
            // both buffers are at least `size` bytes and do not overlap.
            std::ptr::copy_nonoverlapping(device.as_ptr(), host, size);
        }
    }

    fn copy_d2d(&self, src: NonNull<u8>, dst: NonNull<u8>, size: usize) {
        unsafe {
            // SAFETY: both src and dst are NonNull from allocate(), caller guarantees
            // both buffers are at least `size` bytes and do not overlap.
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_ptr(), size);
        }
    }

    fn synchronize(&self) {
        // CPU is synchronous, nothing to do
    }

    fn binop(
        &self,
        a: &TensorHandle,
        b: &TensorHandle,
        op: TensorBinaryOp,
    ) -> Option<TensorHandle> {
        super::cpu::binop_f32_scalar(a, b, op)
    }

    fn unop(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
        super::cpu::unop_f32_scalar(a, op)
    }

    fn reduce(
        &self,
        a: &TensorHandle,
        op: TensorReduceOp,
        axis: Option<usize>,
    ) -> Option<TensorHandle> {
        super::cpu::reduce_f32_scalar(a, op, axis)
    }

    fn matmul(&self, a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
        super::cpu::matmul_f32_scalar(a, b)
    }

    fn memory_info(&self) -> (usize, usize) {
        // Estimate available system memory
        let total = 16 * 1024 * 1024 * 1024; // 16 GB default
        let free = total / 2; // Assume 50% available
        (free, total)
    }
}

// ============================================================================
// Global Backend Registry
// ============================================================================

use std::sync::OnceLock;
use std::collections::HashMap;
use std::sync::Arc;

/// Global backend registry
static BACKEND_REGISTRY: OnceLock<BackendRegistry> = OnceLock::new();

/// Registry of available backends
pub struct BackendRegistry {
    backends: HashMap<DeviceId, Arc<dyn Backend>>,
    default_backend: DeviceId,
    memory_pools: HashMap<DeviceId, MemoryPool>,
}

impl BackendRegistry {
    /// Initialize backend registry
    pub fn init() -> Self {
        let mut backends: HashMap<DeviceId, Arc<dyn Backend>> = HashMap::new();
        let mut memory_pools = HashMap::new();
        #[allow(unused_mut)]
        let mut default_device = DeviceId::CPU;

        // Always add CPU backend
        let cpu_backend = Arc::new(CpuBackend::new());
        backends.insert(DeviceId::CPU, cpu_backend);
        memory_pools.insert(DeviceId::CPU, MemoryPool::new(DeviceId::CPU));

        // Metal GPU backend (macOS)
        #[cfg(all(target_os = "macos", feature = "metal"))]
        {
            if let Some(metal_backend) = super::metal::MetalBackend::new() {
                let device_id = metal_backend.device_id();
                let backend: Arc<dyn Backend> = Arc::new(metal_backend);
                backends.insert(device_id, backend);
                memory_pools.insert(device_id, MemoryPool::new(device_id));
                // Prefer GPU as default for compute-heavy workloads
                default_device = device_id;
            }
        }

        // CUDA backend would be added here
        // #[cfg(feature = "cuda")]
        // { ... }

        Self {
            backends,
            default_backend: default_device,
            memory_pools,
        }
    }

    /// Get backend for device
    pub fn backend(&self, device: DeviceId) -> Option<&Arc<dyn Backend>> {
        self.backends.get(&device)
    }

    /// Get memory pool for device
    pub fn memory_pool(&self, device: DeviceId) -> Option<&MemoryPool> {
        self.memory_pools.get(&device)
    }

    /// Get default backend
    pub fn default_backend(&self) -> &Arc<dyn Backend> {
        self.backends.get(&self.default_backend).unwrap()
    }

    /// List all available devices
    pub fn devices(&self) -> impl Iterator<Item = DeviceId> + '_ {
        self.backends.keys().copied()
    }

    /// Check if GPU is available
    pub fn has_gpu(&self) -> bool {
        self.backends.keys().any(|d| d.is_gpu())
    }

    /// Get first available GPU backend
    pub fn gpu_backend(&self) -> Option<&Arc<dyn Backend>> {
        self.backends
            .iter()
            .find(|(id, _)| id.is_gpu())
            .map(|(_, backend)| backend)
    }

    /// Get number of available devices
    pub fn device_count(&self) -> usize {
        self.backends.len()
    }

    /// Get device info summary
    pub fn device_summary(&self) -> Vec<(DeviceId, &'static str, (usize, usize))> {
        self.backends
            .iter()
            .map(|(id, backend)| (*id, backend.name(), backend.memory_info()))
            .collect()
    }
}

/// Get global backend registry
pub fn get_backend_registry() -> &'static BackendRegistry {
    BACKEND_REGISTRY.get_or_init(BackendRegistry::init)
}

/// Get backend for device
pub fn get_backend(device: DeviceId) -> Option<&'static Arc<dyn Backend>> {
    get_backend_registry().backend(device)
}

/// Get default backend
pub fn default_backend() -> &'static Arc<dyn Backend> {
    get_backend_registry().default_backend()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_backend_creation() {
        let backend = CpuBackend::new();
        assert_eq!(backend.name(), "CPU");
        assert!(backend.device_id().is_cpu());
        assert!(backend.capabilities().max_threads >= 1);
    }

    #[test]
    fn test_memory_pool() {
        let pool = MemoryPool::new(DeviceId::CPU);
        let backend = CpuBackend::new();

        // Allocate — `NonNull` already guarantees non-null at the
        // type level, so checking `.is_null()` after `.unwrap()` is
        // tautological (clippy `useless_ptr_null_checks`).  The
        // unwrap above already enforces "allocation succeeded".
        let ptr = pool.allocate(1024, &backend).unwrap();

        // Return to pool
        pool.deallocate(ptr, 1024);

        // Should get from cache
        let ptr2 = pool.allocate(1024, &backend).unwrap();
        assert_eq!(ptr, ptr2); // Same pointer from cache

        let stats = pool.stats();
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.cache_misses, 1);
    }

    #[test]
    fn test_backend_registry() {
        let registry = get_backend_registry();
        assert!(registry.backend(DeviceId::CPU).is_some());
        // Default backend is GPU if available, otherwise CPU
        let default_name = registry.default_backend().name();
        assert!(default_name == "CPU" || default_name == "Metal" || default_name == "CUDA");
    }

    #[test]
    fn test_sync_flags() {
        let flags = SyncFlags::HOST_DIRTY;
        assert!(flags.contains(SyncFlags::HOST_DIRTY));
        assert!(!flags.contains(SyncFlags::DEVICE_DIRTY));

        let synced = SyncFlags::SYNCED;
        assert!(synced.is_empty());
    }

    #[test]
    fn test_memory_pool_stats() {
        let stats = MemoryPoolStats {
            device: DeviceId::CPU,
            allocated: 1024,
            peak: 2048,
            cache_hits: 10,
            cache_misses: 5,
        };
        assert!((stats.hit_rate() - 0.666).abs() < 0.01);
    }
}
