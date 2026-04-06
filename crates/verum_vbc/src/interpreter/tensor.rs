//! Tensor runtime support for the VBC interpreter.
//!
//! This module provides the runtime representation and operations for tensors,
//! enabling ML/AI workloads in the Verum VBC interpreter.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │                         TENSOR REPRESENTATION                           │
//! ├─────────────────────────────────────────────────────────────────────────┤
//! │                                                                         │
//! │  TensorValue (stack) ──► TensorHandle ──► TensorData (heap)            │
//! │                                                                         │
//! │  ┌──────────────────┐   ┌──────────────────┐   ┌───────────────────┐   │
//! │  │ Value (NaN-boxed)│   │ TensorHandle     │   │ TensorData        │   │
//! │  │ TAG_POINTER ─────┼──►│ dtype: DType     │   │ data: *mut u8     │   │
//! │  └──────────────────┘   │ ndim: u8         │   │ capacity: usize   │   │
//! │                         │ shape: [usize;8] │   │ refcount: u32     │   │
//! │                         │ strides: [...]   │   │ device: DeviceId  │   │
//! │                         │ data: *TensorData│   └───────────────────┘   │
//! │                         │ flags: TensorFlags│                          │
//! │                         └──────────────────┘                           │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Memory Layout
//!
//! Tensors use row-major (C-style) layout by default. The strides are computed
//! automatically from the shape.
//!
//! # Thread Safety
//!
//! TensorData uses atomic reference counting for safe sharing across async tasks.
//! Mutation requires exclusive access (enforced at runtime).

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::fmt;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use crate::instruction::{TensorBinaryOp, TensorReduceOp, TensorUnaryOp};

// ============================================================================
// Half-Precision Conversion Helpers
// ============================================================================

/// Convert F32 to F16 bits (IEEE 754 binary16).
/// Rounds to nearest, with ties to even.
#[inline]
fn f32_to_f16_bits(val: f32) -> u16 {
    let bits = val.to_bits();
    let sign = ((bits >> 31) & 1) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x7FFFFF;

    if exp == 0xFF {
        // Inf or NaN
        if mant == 0 {
            (sign << 15) | (0x1F << 10)
        } else {
            (sign << 15) | (0x1F << 10) | 0x200 // Quiet NaN
        }
    } else if exp > 142 {
        // Overflow to infinity
        (sign << 15) | (0x1F << 10)
    } else if exp < 113 {
        // Underflow to zero or denorm
        if exp < 103 {
            sign << 15 // Zero
        } else {
            // Denormalized
            let shift = 125 - exp;
            let m = (mant | 0x800000) >> shift;
            (sign << 15) | ((m >> 13) as u16)
        }
    } else {
        // Normal number
        let new_exp = (exp - 112) as u16;
        let new_mant = (mant >> 13) as u16;
        // Round to nearest even
        let round = (mant >> 12) & 1;
        let sticky = mant & 0xFFF;
        let mut result = (sign << 15) | (new_exp << 10) | new_mant;
        if round == 1 && (sticky != 0 || (new_mant & 1) == 1) {
            result += 1;
        }
        result
    }
}

/// Convert F16 bits to F32.
#[inline]
fn f16_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mant = (bits & 0x3FF) as u32;

    if exp == 0 {
        if mant == 0 {
            f32::from_bits(sign << 31) // Zero
        } else {
            // Denormalized: normalize
            let mut m = mant;
            let mut e = 1u32;
            while (m & 0x400) == 0 {
                m <<= 1;
                e += 1;
            }
            let new_exp = 127 - 15 - e;
            let new_mant = (m & 0x3FF) << 13;
            f32::from_bits((sign << 31) | (new_exp << 23) | new_mant)
        }
    } else if exp == 0x1F {
        // Inf or NaN
        if mant == 0 {
            f32::from_bits((sign << 31) | (0xFF << 23))
        } else {
            f32::from_bits((sign << 31) | (0xFF << 23) | (mant << 13))
        }
    } else {
        // Normal number
        let new_exp = exp + 127 - 15;
        let new_mant = mant << 13;
        f32::from_bits((sign << 31) | (new_exp << 23) | new_mant)
    }
}

/// Convert F32 to BF16 bits.
/// BF16 has same exponent range as F32 but truncated mantissa.
#[inline]
fn f32_to_bf16_bits(val: f32) -> u16 {
    let bits = val.to_bits();
    // Round to nearest even
    let round_bit = (bits >> 15) & 1;
    let sticky = bits & 0x7FFF;
    if round_bit == 1 && (sticky != 0 || ((bits >> 16) & 1) == 1) {
        ((bits >> 16) + 1) as u16
    } else {
        (bits >> 16) as u16
    }
}

/// Convert BF16 bits to F32.
#[inline]
fn bf16_bits_to_f32(bits: u16) -> f32 {
    f32::from_bits((bits as u32) << 16)
}

// ============================================================================
// Data Types
// ============================================================================

// Re-export DType from the crate-level dtype module for consistency.
// This unifies the previously duplicated DType definitions between
// metadata/shape.rs (compile-time) and interpreter/tensor.rs (runtime).
pub use crate::dtype::DType;

// Note: DType is now defined in crate::dtype and re-exported here.
// The shared module provides all functionality: size(), align(), is_float(),
// is_integer(), is_complex(), promote_static(), etc.

// ============================================================================
// Device Types
// ============================================================================

/// Device identifier for tensor placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeviceId(pub u16);

impl DeviceId {
    /// CPU device.
    pub const CPU: DeviceId = DeviceId(0);

    /// First GPU device.
    pub const GPU0: DeviceId = DeviceId(0x1000);

    /// Returns true if this is the CPU.
    #[inline]
    pub fn is_cpu(&self) -> bool {
        self.0 == 0
    }

    /// Returns true if this is a GPU.
    #[inline]
    pub fn is_gpu(&self) -> bool {
        self.0 >= 0x1000
    }
}

// ============================================================================
// Tensor Flags
// ============================================================================

bitflags::bitflags! {
    /// Flags controlling tensor behavior.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TensorFlags: u16 {
        /// Tensor data is contiguous in memory.
        const CONTIGUOUS = 0x0001;
        /// Tensor owns its data (will free on drop).
        const OWNS_DATA = 0x0002;
        /// Tensor is a view into another tensor.
        const IS_VIEW = 0x0004;
        /// Tensor data is read-only.
        const READ_ONLY = 0x0008;
        /// Tensor requires gradient computation.
        const REQUIRES_GRAD = 0x0010;
        /// Tensor is a leaf in the computation graph.
        const IS_LEAF = 0x0020;
        /// Tensor data is pinned (page-locked for GPU transfer).
        const PINNED = 0x0040;
    }
}

// ============================================================================
// Tensor Data (Heap-allocated)
// ============================================================================

/// Raw tensor data storage (heap-allocated, reference-counted).
///
/// Supports both CPU and GPU storage with automatic host/device synchronization.
pub struct TensorData {
    /// Pointer to raw data (host or device memory).
    data: NonNull<u8>,
    /// Capacity in bytes.
    capacity: usize,
    /// Reference count (atomic for thread safety).
    refcount: AtomicU32,
    /// Device where primary data is stored.
    device: DeviceId,
    /// Layout used for allocation.
    layout: Layout,
    /// Pinned host shadow copy for GPU tensors (for async transfers).
    /// None for CPU tensors or when host shadow is not needed.
    host_shadow: Option<NonNull<u8>>,
    /// Synchronization flags for host/device coherence.
    /// - HOST_DIRTY: host data modified, device copy stale
    /// - DEVICE_DIRTY: device data modified, host copy stale
    /// - SYNCED: both copies in sync
    dirty_flags: AtomicU8,
}

/// Synchronization flags for host/device memory coherence.
pub mod sync_flags {
    /// Host data modified, device copy stale
    pub const HOST_DIRTY: u8 = 0x01;
    /// Device data modified, host copy stale
    pub const DEVICE_DIRTY: u8 = 0x02;
    /// Both copies in sync
    pub const SYNCED: u8 = 0x00;
}

impl TensorData {
    /// Maximum single tensor allocation size (1 GB).
    const MAX_TENSOR_ALLOC: usize = 1024 * 1024 * 1024;

    /// Allocates new tensor data with the given capacity.
    pub fn alloc(capacity: usize, align: usize, device: DeviceId) -> Option<NonNull<Self>> {
        if capacity == 0 {
            return None;
        }

        // Guard against unbounded allocation (DoS prevention)
        if capacity > Self::MAX_TENSOR_ALLOC {
            return None;
        }

        // Only CPU allocation for now
        if !device.is_cpu() {
            return None;
        }

        let layout = Layout::from_size_align(capacity, align).ok()?;

        // SAFETY: We check capacity > 0 and alignment is valid
        let data_ptr = unsafe { alloc_zeroed(layout) };
        if data_ptr.is_null() {
            return None;
        }

        // Allocate the TensorData struct
        let td_layout = Layout::new::<TensorData>();
        let td_ptr = unsafe { alloc(td_layout) as *mut TensorData };
        if td_ptr.is_null() {
            unsafe { dealloc(data_ptr, layout) };
            return None;
        }

        // Initialize
        unsafe {
            td_ptr.write(TensorData {
                data: NonNull::new_unchecked(data_ptr),
                capacity,
                refcount: AtomicU32::new(1),
                device,
                layout,
                host_shadow: None, // No shadow for CPU tensors
                dirty_flags: AtomicU8::new(sync_flags::SYNCED),
            });
        }

        NonNull::new(td_ptr)
    }

    /// Returns the raw data pointer.
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// Returns the raw data pointer (mutable).
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_ptr()
    }

    /// Returns the capacity in bytes.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Increments the reference count.
    #[inline]
    pub fn incref(&self) {
        self.refcount.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements the reference count. Returns true if count reaches zero.
    #[inline]
    pub fn decref(&self) -> bool {
        self.refcount.fetch_sub(1, Ordering::Release) == 1
    }

    /// Returns the current reference count.
    #[inline]
    pub fn refcount(&self) -> u32 {
        self.refcount.load(Ordering::Relaxed)
    }

    /// Returns the device where data is stored.
    #[inline]
    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// Check if data is on CPU.
    #[inline]
    pub fn is_cpu(&self) -> bool {
        self.device.is_cpu()
    }

    /// Check if data is on GPU.
    #[inline]
    pub fn is_gpu(&self) -> bool {
        self.device.is_gpu()
    }

    /// Get current sync flags.
    #[inline]
    pub fn sync_flags(&self) -> u8 {
        self.dirty_flags.load(Ordering::Acquire)
    }

    /// Mark host data as dirty (modified).
    #[inline]
    pub fn mark_host_dirty(&self) {
        self.dirty_flags.fetch_or(sync_flags::HOST_DIRTY, Ordering::Release);
    }

    /// Mark device data as dirty (modified).
    #[inline]
    pub fn mark_device_dirty(&self) {
        self.dirty_flags.fetch_or(sync_flags::DEVICE_DIRTY, Ordering::Release);
    }

    /// Clear dirty flags (data is synced).
    #[inline]
    pub fn mark_synced(&self) {
        self.dirty_flags.store(sync_flags::SYNCED, Ordering::Release);
    }

    /// Check if host copy needs sync from device.
    #[inline]
    pub fn needs_host_sync(&self) -> bool {
        (self.sync_flags() & sync_flags::DEVICE_DIRTY) != 0
    }

    /// Check if device copy needs sync from host.
    #[inline]
    pub fn needs_device_sync(&self) -> bool {
        (self.sync_flags() & sync_flags::HOST_DIRTY) != 0
    }

    /// Get host shadow pointer (for GPU tensors).
    #[inline]
    pub fn host_shadow(&self) -> Option<NonNull<u8>> {
        self.host_shadow
    }
}

impl Drop for TensorData {
    fn drop(&mut self) {
        // Only deallocate if refcount is zero
        if self.refcount.load(Ordering::Acquire) == 0 {
            unsafe {
                // Deallocate main data
                dealloc(self.data.as_ptr(), self.layout);

                // Deallocate host shadow if present
                if let Some(shadow) = self.host_shadow {
                    dealloc(shadow.as_ptr(), self.layout);
                }
            }
        }
    }
}

// ============================================================================
// Tensor Handle
// ============================================================================

/// Maximum number of dimensions supported.
pub const MAX_DIMS: usize = 8;

/// Tensor handle containing metadata and pointer to data.
///
/// This structure is designed to fit in 128 bytes for cache efficiency.
#[repr(C)]
pub struct TensorHandle {
    /// Element data type.
    pub dtype: DType,
    /// Number of dimensions (0-8).
    pub ndim: u8,
    /// Tensor flags.
    pub flags: TensorFlags,
    /// Reserved for alignment.
    _reserved: u16,
    /// Total number of elements.
    pub numel: usize,
    /// Shape (dimensions).
    pub shape: [usize; MAX_DIMS],
    /// Strides in elements (not bytes).
    /// Uses signed integers to support negative strides for reverse iteration,
    /// flip operations, and semantic alignment with core/tensor.vr which uses ISize.
    pub strides: [isize; MAX_DIMS],
    /// Pointer to tensor data.
    pub data: Option<NonNull<TensorData>>,
    /// Offset into data (for views).
    pub offset: usize,
}

impl Default for TensorHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl TensorHandle {
    /// Creates a new empty tensor handle.
    pub fn new() -> Self {
        Self {
            dtype: DType::F32,
            ndim: 0,
            flags: TensorFlags::CONTIGUOUS | TensorFlags::OWNS_DATA,
            _reserved: 0,
            numel: 0,
            shape: [0; MAX_DIMS],
            strides: [0isize; MAX_DIMS],
            data: None,
            offset: 0,
        }
    }

    /// Creates a tensor with the given shape and dtype.
    pub fn zeros(shape: &[usize], dtype: DType) -> Option<Self> {
        if shape.len() > MAX_DIMS {
            return None;
        }

        let numel: usize = shape.iter().product();
        if numel == 0 {
            return Some(Self::new());
        }

        let capacity = numel * dtype.size();
        let data = TensorData::alloc(capacity, dtype.align(), DeviceId::CPU)?;

        let mut handle = Self {
            dtype,
            ndim: shape.len() as u8,
            flags: TensorFlags::CONTIGUOUS | TensorFlags::OWNS_DATA,
            _reserved: 0,
            numel,
            shape: [0; MAX_DIMS],
            strides: [0isize; MAX_DIMS],
            data: Some(data),
            offset: 0,
        };

        // Copy shape and compute strides (row-major)
        for (i, &dim) in shape.iter().enumerate() {
            handle.shape[i] = dim;
        }
        handle.compute_strides();

        Some(handle)
    }

    /// Creates a tensor with the given shape, dtype, and device.
    ///
    /// This is the device-aware version of `zeros`. It respects the
    /// ComputeDevice context when allocating tensor storage.
    ///
    /// # Arguments
    /// * `shape` - The dimensions of the tensor
    /// * `dtype` - The element data type
    /// * `device` - The device ID (0 = CPU, 0x1000+ = GPU)
    ///
    /// # Returns
    /// `Some(TensorHandle)` on success, `None` if allocation fails or
    /// the device is not available.
    pub fn zeros_on(shape: &[usize], dtype: DType, device: DeviceId) -> Option<Self> {
        if shape.len() > MAX_DIMS {
            return None;
        }

        let numel: usize = shape.iter().product();
        if numel == 0 {
            return Some(Self::new());
        }

        let capacity = numel * dtype.size();
        let data = TensorData::alloc(capacity, dtype.align(), device)?;

        let mut handle = Self {
            dtype,
            ndim: shape.len() as u8,
            flags: TensorFlags::CONTIGUOUS | TensorFlags::OWNS_DATA,
            _reserved: 0,
            numel,
            shape: [0; MAX_DIMS],
            strides: [0isize; MAX_DIMS],
            data: Some(data),
            offset: 0,
        };

        // Copy shape and compute strides (row-major)
        for (i, &dim) in shape.iter().enumerate() {
            handle.shape[i] = dim;
        }
        handle.compute_strides();

        Some(handle)
    }

    /// Creates a tensor filled with a constant value.
    pub fn full(shape: &[usize], dtype: DType, value: f64) -> Option<Self> {
        Self::full_on(shape, dtype, value, DeviceId::CPU)
    }

    /// Creates a tensor filled with a constant value on a specific device.
    ///
    /// # Arguments
    /// * `shape` - The dimensions of the tensor
    /// * `dtype` - The element data type
    /// * `value` - The fill value (cast to the appropriate dtype)
    /// * `device` - The device ID (0 = CPU, 0x1000+ = GPU)
    pub fn full_on(shape: &[usize], dtype: DType, value: f64, device: DeviceId) -> Option<Self> {
        let handle = Self::zeros_on(shape, dtype, device)?;

        if let Some(data) = &handle.data {
            unsafe {
                let ptr = (*data.as_ptr()).as_mut_ptr();
                handle.fill_with_value(ptr, value);
            }
        }

        Some(handle)
    }

    /// Creates a tensor from raw data.
    ///
    /// # Safety
    ///
    /// The caller must ensure the data is valid and has the correct size.
    pub unsafe fn from_raw(
        shape: &[usize],
        dtype: DType,
        data_ptr: *const u8,
    ) -> Option<Self> {
        let handle = Self::zeros(shape, dtype)?;

        if let Some(data) = &handle.data {
            // SAFETY: data pointer is valid by construction, data_ptr guaranteed by caller
            unsafe {
                let dst = (*data.as_ptr()).as_mut_ptr();
                let size = handle.numel * dtype.size();
                std::ptr::copy_nonoverlapping(data_ptr, dst, size);
            }
        }

        Some(handle)
    }

    /// Computes strides for row-major (C-style) layout.
    /// Produces signed strides for semantic alignment with core/tensor.vr.
    fn compute_strides(&mut self) {
        if self.ndim == 0 {
            return;
        }

        let mut stride = 1isize;
        for i in (0..self.ndim as usize).rev() {
            self.strides[i] = stride;
            stride *= self.shape[i] as isize;
        }
    }

    /// Fills the tensor with a constant value.
    unsafe fn fill_with_value(&self, ptr: *mut u8, value: f64) {
        // SAFETY: Caller ensures ptr is valid for numel elements
        unsafe {
            match self.dtype {
                DType::F32 => {
                    let p = ptr as *mut f32;
                    let v = value as f32;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::F64 => {
                    let p = ptr as *mut f64;
                    for i in 0..self.numel {
                        *p.add(i) = value;
                    }
                }
                DType::I8 => {
                    let p = ptr as *mut i8;
                    let v = value as i8;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::I16 => {
                    let p = ptr as *mut i16;
                    let v = value as i16;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::I32 => {
                    let p = ptr as *mut i32;
                    let v = value as i32;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::I64 => {
                    let p = ptr as *mut i64;
                    let v = value as i64;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::U8 => {
                    let v = value as u8;
                    for i in 0..self.numel {
                        *ptr.add(i) = v;
                    }
                }
                DType::U16 => {
                    let p = ptr as *mut u16;
                    let v = value as u16;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::U32 => {
                    let p = ptr as *mut u32;
                    let v = value as u32;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::U64 => {
                    let p = ptr as *mut u64;
                    let v = value as u64;
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::Bool => {
                    let v = if value != 0.0 { 1u8 } else { 0u8 };
                    for i in 0..self.numel {
                        *ptr.add(i) = v;
                    }
                }
                DType::F16 => {
                    let p = ptr as *mut u16;
                    // Convert f64 -> f32 -> f16 bits
                    let v = f32_to_f16_bits(value as f32);
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::BF16 => {
                    let p = ptr as *mut u16;
                    // Convert f64 -> f32 -> bf16 bits
                    let v = f32_to_bf16_bits(value as f32);
                    for i in 0..self.numel {
                        *p.add(i) = v;
                    }
                }
                DType::Complex64 => {
                    let p = ptr as *mut f32;
                    let v = value as f32;
                    for i in 0..self.numel {
                        *p.add(i * 2) = v;     // real part
                        *p.add(i * 2 + 1) = 0.0; // imag part
                    }
                }
                DType::Complex128 => {
                    let p = ptr as *mut f64;
                    for i in 0..self.numel {
                        *p.add(i * 2) = value;   // real part
                        *p.add(i * 2 + 1) = 0.0; // imag part
                    }
                }
            }
        }
    }

    /// Returns true if the tensor is contiguous in memory.
    #[inline]
    pub fn is_contiguous(&self) -> bool {
        self.flags.contains(TensorFlags::CONTIGUOUS)
    }

    /// Returns the total size in bytes.
    #[inline]
    pub fn size_bytes(&self) -> usize {
        self.numel * self.dtype.size()
    }

    /// Returns a pointer to element at the given indices.
    ///
    /// # Safety
    ///
    /// The caller must ensure indices are valid.
    /// Supports negative strides for reverse iteration (e.g., flip operations).
    pub unsafe fn get_ptr(&self, indices: &[usize]) -> Option<*const u8> {
        let data = self.data.as_ref()?;
        // SAFETY: data pointer is guaranteed valid by construction
        let base = unsafe { (*data.as_ptr()).as_ptr() };

        // Use signed arithmetic to support negative strides
        let mut offset = self.offset as isize;
        for (i, &idx) in indices.iter().enumerate() {
            if idx >= self.shape[i] {
                return None;
            }
            offset += (idx as isize) * self.strides[i];
        }

        // SAFETY: offset is computed from valid indices; negative strides are only
        // valid when base pointer has been adjusted to point to the logical start
        debug_assert!(offset >= 0, "Negative offset from indexing indicates invalid view");
        Some(unsafe { base.add((offset as usize) * self.dtype.size()) })
    }

    /// Gets a scalar value from the tensor (for scalar tensors).
    pub fn get_scalar_f64(&self) -> Option<f64> {
        if self.numel != 1 {
            return None;
        }

        let data = self.data.as_ref()?;
        unsafe {
            let ptr = (*data.as_ptr()).as_ptr();
            match self.dtype {
                DType::F32 => Some(*(ptr as *const f32) as f64),
                DType::F64 => Some(*(ptr as *const f64)),
                DType::I8 => Some(*(ptr as *const i8) as f64),
                DType::I16 => Some(*(ptr as *const i16) as f64),
                DType::I32 => Some(*(ptr as *const i32) as f64),
                DType::I64 => Some(*(ptr as *const i64) as f64),
                DType::U8 => Some(*ptr as f64),
                DType::U16 => Some(*(ptr as *const u16) as f64),
                DType::U32 => Some(*(ptr as *const u32) as f64),
                DType::U64 => Some(*(ptr as *const u64) as f64),
                DType::Bool => Some(if *ptr != 0 { 1.0 } else { 0.0 }),
                DType::F16 => Some(f16_bits_to_f32(*(ptr as *const u16)) as f64),
                DType::BF16 => Some(bf16_bits_to_f32(*(ptr as *const u16)) as f64),
                DType::Complex64 => Some(*(ptr as *const f32) as f64), // Returns real part
                DType::Complex128 => Some(*(ptr as *const f64)),       // Returns real part
            }
        }
    }

    /// Gets element at flat index as f64.
    /// Unlike `get_scalar_f64`, this works for multi-element tensors.
    pub fn get_element_f64(&self, index: usize) -> Option<f64> {
        if index >= self.numel {
            return None;
        }
        let data = self.data.as_ref()?;
        unsafe {
            let ptr = (*data.as_ptr()).as_ptr();
            match self.dtype {
                DType::F32 => Some(*((ptr as *const f32).add(index)) as f64),
                DType::F64 => Some(*((ptr as *const f64).add(index))),
                DType::I8 => Some(*((ptr as *const i8).add(index)) as f64),
                DType::I16 => Some(*((ptr as *const i16).add(index)) as f64),
                DType::I32 => Some(*((ptr as *const i32).add(index)) as f64),
                DType::I64 => Some(*((ptr as *const i64).add(index)) as f64),
                DType::U8 => Some(*(ptr.add(index)) as f64),
                DType::U16 => Some(*((ptr as *const u16).add(index)) as f64),
                DType::U32 => Some(*((ptr as *const u32).add(index)) as f64),
                DType::U64 => Some(*((ptr as *const u64).add(index)) as f64),
                DType::Bool => Some(if *(ptr.add(index)) != 0 { 1.0 } else { 0.0 }),
                DType::F16 => Some(f16_bits_to_f32(*((ptr as *const u16).add(index))) as f64),
                DType::BF16 => Some(bf16_bits_to_f32(*((ptr as *const u16).add(index))) as f64),
                DType::Complex64 => Some(*((ptr as *const f32).add(index * 2)) as f64),
                DType::Complex128 => Some(*((ptr as *const f64).add(index * 2))),
            }
        }
    }

    /// Increments the data reference count.
    pub fn incref(&self) {
        if let Some(data) = &self.data {
            unsafe { (*data.as_ptr()).incref() }
        }
    }

    /// Decrements the data reference count and frees if zero.
    pub fn decref(&mut self) {
        if let Some(data) = self.data.take() {
            unsafe {
                if (*data.as_ptr()).decref() {
                    // Free the TensorData
                    let layout = Layout::new::<TensorData>();
                    std::ptr::drop_in_place(data.as_ptr());
                    dealloc(data.as_ptr() as *mut u8, layout);
                }
            }
        }
    }

    // =========================================================================
    // Raw pointer accessors for kernel dispatch
    // =========================================================================

    /// Returns raw pointer to F32 data (const).
    ///
    /// Returns null if no data or wrong dtype.
    #[inline]
    pub fn data_ptr_f32(&self) -> *const f32 {
        if self.dtype != DType::F32 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const f32 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to F32 data (mutable).
    ///
    /// Returns null if no data or wrong dtype.
    #[inline]
    pub fn data_ptr_f32_mut(&mut self) -> *mut f32 {
        if self.dtype != DType::F32 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut f32 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to F64 data (const).
    #[inline]
    pub fn data_ptr_f64(&self) -> *const f64 {
        if self.dtype != DType::F64 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const f64 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to F64 data (mutable).
    #[inline]
    pub fn data_ptr_f64_mut(&mut self) -> *mut f64 {
        if self.dtype != DType::F64 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut f64 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to I32 data (const).
    #[inline]
    pub fn data_ptr_i32(&self) -> *const i32 {
        if self.dtype != DType::I32 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const i32 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to I32 data (mutable).
    #[inline]
    pub fn data_ptr_i32_mut(&mut self) -> *mut i32 {
        if self.dtype != DType::I32 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut i32 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to I64 data (const).
    #[inline]
    pub fn data_ptr_i64(&self) -> *const i64 {
        if self.dtype != DType::I64 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const i64 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to I64 data (mutable).
    #[inline]
    pub fn data_ptr_i64_mut(&mut self) -> *mut i64 {
        if self.dtype != DType::I64 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut i64 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to I16 data (const).
    #[inline]
    pub fn data_ptr_i16(&self) -> *const i16 {
        if self.dtype != DType::I16 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const i16 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to I16 data (mutable).
    #[inline]
    pub fn data_ptr_i16_mut(&mut self) -> *mut i16 {
        if self.dtype != DType::I16 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut i16 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to I8 data (const).
    #[inline]
    pub fn data_ptr_i8(&self) -> *const i8 {
        if self.dtype != DType::I8 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const i8 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to I8 data (mutable).
    #[inline]
    pub fn data_ptr_i8_mut(&mut self) -> *mut i8 {
        if self.dtype != DType::I8 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut i8 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to U32 data (const).
    #[inline]
    pub fn data_ptr_u32(&self) -> *const u32 {
        if self.dtype != DType::U32 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const u32 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to U32 data (mutable).
    #[inline]
    pub fn data_ptr_u32_mut(&mut self) -> *mut u32 {
        if self.dtype != DType::U32 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut u32 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to U64 data (const).
    #[inline]
    pub fn data_ptr_u64(&self) -> *const u64 {
        if self.dtype != DType::U64 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const u64 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to U64 data (mutable).
    #[inline]
    pub fn data_ptr_u64_mut(&mut self) -> *mut u64 {
        if self.dtype != DType::U64 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut u64 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to U8 data (const).
    #[inline]
    pub fn data_ptr_u8(&self) -> *const u8 {
        if self.dtype != DType::U8 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to U8 data (mutable).
    #[inline]
    pub fn data_ptr_u8_mut(&mut self) -> *mut u8 {
        if self.dtype != DType::U8 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to U16 data (const).
    #[inline]
    pub fn data_ptr_u16(&self) -> *const u16 {
        if self.dtype != DType::U16 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const u16 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to U16 data (mutable).
    #[inline]
    pub fn data_ptr_u16_mut(&mut self) -> *mut u16 {
        if self.dtype != DType::U16 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut u16 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to F16 data as raw u16 bits (const).
    ///
    /// F16 (IEEE 754 binary16) stores 16-bit half-precision floats.
    /// Use f16_to_f32 and f32_to_f16 for conversions.
    #[inline]
    pub fn data_ptr_f16(&self) -> *const u16 {
        if self.dtype != DType::F16 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const u16 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to F16 data as raw u16 bits (mutable).
    #[inline]
    pub fn data_ptr_f16_mut(&mut self) -> *mut u16 {
        if self.dtype != DType::F16 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut u16 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to BF16 data as raw u16 bits (const).
    ///
    /// BF16 (Brain float16) stores 16-bit floats with same exponent range as f32.
    /// Use bf16_to_f32 and f32_to_bf16 for conversions.
    #[inline]
    pub fn data_ptr_bf16(&self) -> *const u16 {
        if self.dtype != DType::BF16 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const u16 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to BF16 data as raw u16 bits (mutable).
    #[inline]
    pub fn data_ptr_bf16_mut(&mut self) -> *mut u16 {
        if self.dtype != DType::BF16 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut u16 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to Complex64 data as f32 pairs (const).
    ///
    /// Complex64 is stored as interleaved pairs of f32: [re0, im0, re1, im1, ...]
    /// Each complex number uses 8 bytes (2 × f32).
    #[inline]
    pub fn data_ptr_complex64(&self) -> *const f32 {
        if self.dtype != DType::Complex64 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const f32 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to Complex64 data as f32 pairs (mutable).
    #[inline]
    pub fn data_ptr_complex64_mut(&mut self) -> *mut f32 {
        if self.dtype != DType::Complex64 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut f32 })
            .unwrap_or(std::ptr::null_mut())
    }

    /// Returns raw pointer to Complex128 data as f64 pairs (const).
    ///
    /// Complex128 is stored as interleaved pairs of f64: [re0, im0, re1, im1, ...]
    /// Each complex number uses 16 bytes (2 × f64).
    #[inline]
    pub fn data_ptr_complex128(&self) -> *const f64 {
        if self.dtype != DType::Complex128 {
            return std::ptr::null();
        }
        self.data.as_ref()
            .map(|d| unsafe { (*d.as_ptr()).as_ptr() as *const f64 })
            .unwrap_or(std::ptr::null())
    }

    /// Returns raw pointer to Complex128 data as f64 pairs (mutable).
    #[inline]
    pub fn data_ptr_complex128_mut(&mut self) -> *mut f64 {
        if self.dtype != DType::Complex128 {
            return std::ptr::null_mut();
        }
        self.data.as_mut()
            .map(|d| unsafe { (*d.as_ptr()).as_mut_ptr() as *mut f64 })
            .unwrap_or(std::ptr::null_mut())
    }
}

impl Clone for TensorHandle {
    fn clone(&self) -> Self {
        // Shallow clone - just increment refcount
        self.incref();

        Self {
            dtype: self.dtype,
            ndim: self.ndim,
            flags: self.flags & !TensorFlags::OWNS_DATA, // Clone doesn't own
            _reserved: 0,
            numel: self.numel,
            shape: self.shape,
            strides: self.strides,
            data: self.data,
            offset: self.offset,
        }
    }
}

impl Drop for TensorHandle {
    fn drop(&mut self) {
        if self.flags.contains(TensorFlags::OWNS_DATA) {
            self.decref();
        }
    }
}

impl fmt::Debug for TensorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shape_slice: &[usize] = &self.shape[..self.ndim as usize];
        f.debug_struct("TensorHandle")
            .field("dtype", &self.dtype)
            .field("shape", &shape_slice)
            .field("numel", &self.numel)
            .field("flags", &self.flags)
            .finish()
    }
}

// ============================================================================
// Tensor Operations
// ============================================================================

/// Performs element-wise binary operation with broadcasting and type promotion.
///
/// Type promotion follows NumPy conventions:
/// - Bool → Integer → Float → Complex
/// - Smaller types promote to larger within category
///
/// Uses SIMD-accelerated scalar broadcasting when one operand has numel=1.
pub fn tensor_binop(
    lhs: &TensorHandle,
    rhs: &TensorHandle,
    op: TensorBinaryOp,
) -> Option<TensorHandle> {
    // Type promotion: compute common dtype
    let result_dtype = DType::promote_static(lhs.dtype, rhs.dtype);

    // Cast operands to common dtype if needed
    let lhs_promoted;
    let rhs_promoted;

    let lhs_ref = if lhs.dtype != result_dtype {
        lhs_promoted = tensor_cast(lhs, result_dtype)?;
        &lhs_promoted
    } else {
        lhs
    };

    let rhs_ref = if rhs.dtype != result_dtype {
        rhs_promoted = tensor_cast(rhs, result_dtype)?;
        &rhs_promoted
    } else {
        rhs
    };

    // Check shape compatibility
    let lhs_shape = &lhs_ref.shape[..lhs_ref.ndim as usize];
    let rhs_shape = &rhs_ref.shape[..rhs_ref.ndim as usize];

    // Verify broadcast compatibility
    let _out_shape = super::kernel::broadcast_shapes(lhs_shape, rhs_shape)?;

    // Use optimized dispatch with scalar broadcast support
    // This handles:
    // 1. Same shapes → direct SIMD kernel
    // 2. One scalar (numel=1) → SIMD splat without memory expansion
    // 3. General broadcast → expand then operate
    super::kernel::dispatch_binop_broadcast(lhs_ref, rhs_ref, op)
}

/// Performs element-wise unary operation.
/// Unary operation on a tensor.
/// Uses kernel dispatch for SIMD (AVX2/NEON) and GPU (Metal) acceleration.
pub fn tensor_unop(src: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
    // Use kernel dispatch for optimized execution (SIMD/GPU)
    if let Some(result) = super::kernel::dispatch_unop(src, op) {
        return Some(result);
    }

    // Fallback to manual implementation if dispatch fails
    let result = TensorHandle::zeros(&src.shape[..src.ndim as usize], src.dtype)?;

    let src_data = src.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let out_ptr = (*out_data.as_ptr()).as_mut_ptr();

        match src.dtype {
            DType::F32 => {
                tensor_unop_f32(src_ptr as *const f32, out_ptr as *mut f32, result.numel, op);
            }
            DType::F64 => {
                tensor_unop_f64(src_ptr as *const f64, out_ptr as *mut f64, result.numel, op);
            }
            _ => return None,
        }
    }

    Some(result)
}

/// F32 unary operations.
unsafe fn tensor_unop_f32(src: *const f32, out: *mut f32, n: usize, op: TensorUnaryOp) {
    // SAFETY: Caller must ensure src and out are valid for n elements
    unsafe {
        for i in 0..n {
            let v = *src.add(i);
            *out.add(i) = match op {
                TensorUnaryOp::Neg => -v,
                TensorUnaryOp::Abs => v.abs(),
                TensorUnaryOp::Sqrt => v.sqrt(),
                TensorUnaryOp::Exp => v.exp(),
                TensorUnaryOp::Log => v.ln(),
                TensorUnaryOp::Sin => v.sin(),
                TensorUnaryOp::Cos => v.cos(),
                TensorUnaryOp::Tan => v.tan(),
                TensorUnaryOp::Tanh => v.tanh(),
                TensorUnaryOp::Sigmoid => 1.0 / (1.0 + (-v).exp()),
                TensorUnaryOp::Relu => v.max(0.0),
                TensorUnaryOp::Floor => v.floor(),
                TensorUnaryOp::Ceil => v.ceil(),
                TensorUnaryOp::Round => v.round(),
                TensorUnaryOp::Sign => {
                    if v > 0.0 {
                        1.0
                    } else if v < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                TensorUnaryOp::Rsqrt => 1.0 / v.sqrt(),
                TensorUnaryOp::Erf => {
                    // Error function approximation (Abramowitz-Stegun)
                    let x = v;
                    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
                    let y = 1.0
                        - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_8) * t
                            - 0.284_496_72)
                            * t
                            + 0.254_829_6)
                            * t
                            * (-x * x).exp();
                    if x >= 0.0 { y } else { -y }
                }
                TensorUnaryOp::Log2 => v.log2(),
                TensorUnaryOp::Softplus => (1.0 + v.exp()).ln(),
                TensorUnaryOp::Mish => v * (1.0 + v.exp()).ln().tanh(),
                TensorUnaryOp::Gelu => {
                    // GELU approximation: 0.5 * x * (1 + tanh(sqrt(2/pi) * (x + 0.044715 * x^3)))
                    let x = v;
                    0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044715 * x * x * x)).tanh())
                }
                TensorUnaryOp::Silu => v * (1.0 / (1.0 + (-v).exp())), // x * sigmoid(x)
            };
        }
    }
}

/// F64 unary operations.
unsafe fn tensor_unop_f64(src: *const f64, out: *mut f64, n: usize, op: TensorUnaryOp) {
    // SAFETY: Caller must ensure src and out are valid for n elements
    unsafe {
        for i in 0..n {
            let v = *src.add(i);
            *out.add(i) = match op {
                TensorUnaryOp::Neg => -v,
                TensorUnaryOp::Abs => v.abs(),
                TensorUnaryOp::Sqrt => v.sqrt(),
                TensorUnaryOp::Exp => v.exp(),
                TensorUnaryOp::Log => v.ln(),
                TensorUnaryOp::Sin => v.sin(),
                TensorUnaryOp::Cos => v.cos(),
                TensorUnaryOp::Tan => v.tan(),
                TensorUnaryOp::Tanh => v.tanh(),
                TensorUnaryOp::Sigmoid => 1.0 / (1.0 + (-v).exp()),
                TensorUnaryOp::Relu => v.max(0.0),
                TensorUnaryOp::Floor => v.floor(),
                TensorUnaryOp::Ceil => v.ceil(),
                TensorUnaryOp::Round => v.round(),
                TensorUnaryOp::Sign => {
                    if v > 0.0 {
                        1.0
                    } else if v < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                TensorUnaryOp::Rsqrt => 1.0 / v.sqrt(),
                TensorUnaryOp::Erf => {
                    // Error function approximation
                    let x = v;
                    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
                    let y = 1.0
                        - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t
                            - 0.284496736)
                            * t
                            + 0.254829592)
                            * t
                            * (-x * x).exp();
                    if x >= 0.0 { y } else { -y }
                }
                TensorUnaryOp::Log2 => v.log2(),
                TensorUnaryOp::Softplus => (1.0 + v.exp()).ln(),
                TensorUnaryOp::Mish => v * (1.0 + v.exp()).ln().tanh(),
                TensorUnaryOp::Gelu => {
                    let x = v;
                    0.5 * x * (1.0 + (0.7978845608028654 * (x + 0.044715 * x * x * x)).tanh())
                }
                TensorUnaryOp::Silu => v * (1.0 / (1.0 + (-v).exp())),
            };
        }
    }
}

/// Performs reduction operation.
/// Uses kernel dispatch for SIMD (AVX2/NEON) acceleration on full reductions.
pub fn tensor_reduce(
    src: &TensorHandle,
    axis: Option<usize>,
    op: TensorReduceOp,
) -> Option<TensorHandle> {
    // Full reduction (all elements -> scalar)
    if axis.is_none() {
        // Try kernel dispatch for optimized SIMD execution
        if let Some(result) = super::kernel::dispatch_reduce(src, op, axis) {
            return Some(result);
        }

        // Fallback to manual implementation
        let result_shape: &[usize] = &[];
        let mut result = TensorHandle::zeros(result_shape, src.dtype)?;

        let src_data = src.data.as_ref()?;

        unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr();
            let value = match src.dtype {
                DType::F32 => reduce_f32(src_ptr as *const f32, src.numel, op),
                DType::F64 => reduce_f64(src_ptr as *const f64, src.numel, op),
                _ => return None,
            };

            // Store result as scalar
            if let Some(data) = &result.data {
                let out_ptr = (*data.as_ptr()).as_mut_ptr();
                match result.dtype {
                    DType::F32 => *(out_ptr as *mut f32) = value as f32,
                    DType::F64 => *(out_ptr as *mut f64) = value,
                    _ => {}
                }
            }
            result.numel = 1;
            result.ndim = 0;
        }

        return Some(result);
    }

    // Axis-specific reduction
    let axis = axis.unwrap();
    if axis >= src.ndim as usize {
        return None;
    }

    // Create result shape by removing the axis dimension
    let mut result_shape = Vec::with_capacity(src.ndim as usize - 1);
    for i in 0..src.ndim as usize {
        if i != axis {
            result_shape.push(src.shape[i]);
        }
    }

    let result = TensorHandle::zeros(&result_shape, src.dtype)?;
    let src_data = src.data.as_ref()?;
    let axis_size = src.shape[axis];

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let out_ptr = if let Some(data) = &result.data {
            (*data.as_ptr()).as_mut_ptr()
        } else {
            return None;
        };

        // Compute strides for indexing
        let src_strides = &src.strides[..src.ndim as usize];
        let axis_stride = src_strides[axis];

        // Total output elements
        let out_numel = result.numel;

        match src.dtype {
            DType::F32 => {
                for out_idx in 0..out_numel {
                    // Convert out_idx to multi-index (skipping axis)
                    // Use signed arithmetic to support negative strides
                    let mut src_base_idx = 0isize;
                    let mut temp = out_idx;
                    let mut dim_cursor = 0;

                    for i in (0..src.ndim as usize).rev() {
                        if i == axis {
                            continue;
                        }
                        let out_dim = result_shape[dim_cursor];
                        let coord = temp % out_dim;
                        temp /= out_dim;
                        src_base_idx += (coord as isize) * src_strides[i];
                        dim_cursor += 1;
                    }

                    // Reduce along axis
                    debug_assert!(src_base_idx >= 0, "Negative base index indicates invalid view");
                    let value = reduce_axis_f32(
                        (src_ptr as *const f32).offset(src_base_idx),
                        axis_size,
                        axis_stride,
                        op,
                    );
                    *(out_ptr as *mut f32).add(out_idx) = value as f32;
                }
            }
            DType::F64 => {
                for out_idx in 0..out_numel {
                    // Convert out_idx to multi-index (skipping axis)
                    // Use signed arithmetic to support negative strides
                    let mut src_base_idx = 0isize;
                    let mut temp = out_idx;
                    let mut dim_cursor = 0;

                    for i in (0..src.ndim as usize).rev() {
                        if i == axis {
                            continue;
                        }
                        let out_dim = result_shape[dim_cursor];
                        let coord = temp % out_dim;
                        temp /= out_dim;
                        src_base_idx += (coord as isize) * src_strides[i];
                        dim_cursor += 1;
                    }

                    // Reduce along axis
                    debug_assert!(src_base_idx >= 0, "Negative base index indicates invalid view");
                    let value = reduce_axis_f64(
                        (src_ptr as *const f64).offset(src_base_idx),
                        axis_size,
                        axis_stride,
                        op,
                    );
                    *(out_ptr as *mut f64).add(out_idx) = value;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Performs reduction operation over multiple axes specified by a bitmask.
///
/// The `axes_mask` is a bitmask where bit `i` indicates that axis `i` should be reduced.
/// - axes_mask = 0: Full reduction (all axes → scalar)
/// - axes_mask = 0b001: Reduce axis 0 only
/// - axes_mask = 0b011: Reduce axes 0 and 1
/// - axes_mask = u64::MAX: Full reduction
///
/// Axes are reduced in descending order to preserve index validity during sequential reduction.
pub fn tensor_reduce_axes(
    src: &TensorHandle,
    axes_mask: u64,
    op: TensorReduceOp,
) -> Option<TensorHandle> {
    // Special case: full reduction (all axes or axes_mask == 0)
    if axes_mask == 0 || axes_mask == u64::MAX || axes_mask.count_ones() == src.ndim as u32 {
        return tensor_reduce(src, None, op);
    }

    // Extract axes from mask into a sorted (descending) list
    // We reduce in descending order so that axis indices remain valid after each reduction
    let mut axes: Vec<usize> = Vec::new();
    for i in 0..src.ndim as u64 {
        if (axes_mask >> i) & 1 == 1 {
            axes.push(i as usize);
        }
    }

    // No axes set - return a clone of the input
    if axes.is_empty() {
        return Some(src.clone());
    }

    // Sort axes in descending order for correct sequential reduction
    axes.sort_by(|a, b| b.cmp(a));

    // Sequentially reduce each axis
    let mut current = src.clone();
    for &axis in &axes {
        // After each reduction, axis indices shift, but since we process
        // in descending order, earlier reductions don't affect later axis indices
        current = tensor_reduce(&current, Some(axis), op)?;
    }

    Some(current)
}

/// Axis-specific F32 reduction with stride.
///
/// # Safety
/// - `src` must point to valid memory with at least `n * stride` elements
/// - All accessed indices must be within bounds
/// - Supports negative strides for reverse iteration
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn reduce_axis_f32(src: *const f32, n: usize, stride: isize, op: TensorReduceOp) -> f64 {
    if n == 0 {
        return 0.0;
    }

    match op {
        TensorReduceOp::Sum => {
            let mut acc = 0.0f64;
            for i in 0..n {
                acc += *src.offset((i as isize) * stride) as f64;
            }
            acc
        }
        TensorReduceOp::Prod => {
            let mut acc = 1.0f64;
            for i in 0..n {
                acc *= *src.offset((i as isize) * stride) as f64;
            }
            acc
        }
        TensorReduceOp::Max => {
            let mut acc = *src as f64;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride) as f64;
                if v > acc {
                    acc = v;
                }
            }
            acc
        }
        TensorReduceOp::Min => {
            let mut acc = *src as f64;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride) as f64;
                if v < acc {
                    acc = v;
                }
            }
            acc
        }
        TensorReduceOp::Mean => {
            let mut acc = 0.0f64;
            for i in 0..n {
                acc += *src.offset((i as isize) * stride) as f64;
            }
            acc / n as f64
        }
        TensorReduceOp::All => {
            for i in 0..n {
                if *src.offset((i as isize) * stride) == 0.0 {
                    return 0.0;
                }
            }
            1.0
        }
        TensorReduceOp::Any => {
            for i in 0..n {
                if *src.offset((i as isize) * stride) != 0.0 {
                    return 1.0;
                }
            }
            0.0
        }
        TensorReduceOp::Std | TensorReduceOp::Var => {
            // Compute mean first
            let mut sum = 0.0f64;
            for i in 0..n {
                sum += *src.offset((i as isize) * stride) as f64;
            }
            let mean = sum / n as f64;

            // Compute variance
            let mut var_sum = 0.0f64;
            for i in 0..n {
                let diff = *src.offset((i as isize) * stride) as f64 - mean;
                var_sum += diff * diff;
            }
            let variance = var_sum / n as f64;

            if matches!(op, TensorReduceOp::Std) {
                variance.sqrt()
            } else {
                variance
            }
        }
        TensorReduceOp::Norm => {
            let mut sum_sq = 0.0f64;
            for i in 0..n {
                let v = *src.offset((i as isize) * stride) as f64;
                sum_sq += v * v;
            }
            sum_sq.sqrt()
        }
        TensorReduceOp::LogSumExp => {
            let mut max_val = *src as f64;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride) as f64;
                if v > max_val {
                    max_val = v;
                }
            }
            let mut sum_exp = 0.0f64;
            for i in 0..n {
                sum_exp += (*src.offset((i as isize) * stride) as f64 - max_val).exp();
            }
            max_val + sum_exp.ln()
        }
    }
}

/// Axis-specific F64 reduction with stride.
///
/// # Safety
/// - `src` must point to valid memory with at least `n * stride` elements
/// - All accessed indices must be within bounds
/// - Supports negative strides for reverse iteration
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn reduce_axis_f64(src: *const f64, n: usize, stride: isize, op: TensorReduceOp) -> f64 {
    if n == 0 {
        return 0.0;
    }

    match op {
        TensorReduceOp::Sum => {
            let mut acc = 0.0f64;
            for i in 0..n {
                acc += *src.offset((i as isize) * stride);
            }
            acc
        }
        TensorReduceOp::Prod => {
            let mut acc = 1.0f64;
            for i in 0..n {
                acc *= *src.offset((i as isize) * stride);
            }
            acc
        }
        TensorReduceOp::Max => {
            let mut acc = *src;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride);
                if v > acc {
                    acc = v;
                }
            }
            acc
        }
        TensorReduceOp::Min => {
            let mut acc = *src;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride);
                if v < acc {
                    acc = v;
                }
            }
            acc
        }
        TensorReduceOp::Mean => {
            let mut acc = 0.0f64;
            for i in 0..n {
                acc += *src.offset((i as isize) * stride);
            }
            acc / n as f64
        }
        TensorReduceOp::All => {
            for i in 0..n {
                if *src.offset((i as isize) * stride) == 0.0 {
                    return 0.0;
                }
            }
            1.0
        }
        TensorReduceOp::Any => {
            for i in 0..n {
                if *src.offset((i as isize) * stride) != 0.0 {
                    return 1.0;
                }
            }
            0.0
        }
        TensorReduceOp::Std | TensorReduceOp::Var => {
            // Compute mean first
            let mut sum = 0.0f64;
            for i in 0..n {
                sum += *src.offset((i as isize) * stride);
            }
            let mean = sum / n as f64;

            // Compute variance
            let mut var_sum = 0.0f64;
            for i in 0..n {
                let diff = *src.offset((i as isize) * stride) - mean;
                var_sum += diff * diff;
            }
            let variance = var_sum / n as f64;

            if matches!(op, TensorReduceOp::Std) {
                variance.sqrt()
            } else {
                variance
            }
        }
        TensorReduceOp::Norm => {
            let mut sum_sq = 0.0f64;
            for i in 0..n {
                let v = *src.offset((i as isize) * stride);
                sum_sq += v * v;
            }
            sum_sq.sqrt()
        }
        TensorReduceOp::LogSumExp => {
            let mut max_val = *src;
            for i in 1..n {
                let v = *src.offset((i as isize) * stride);
                if v > max_val {
                    max_val = v;
                }
            }
            let mut sum_exp = 0.0f64;
            for i in 0..n {
                sum_exp += (*src.offset((i as isize) * stride) - max_val).exp();
            }
            max_val + sum_exp.ln()
        }
    }
}

/// F32 reduction.
unsafe fn reduce_f32(src: *const f32, n: usize, op: TensorReduceOp) -> f64 {
    if n == 0 {
        return 0.0;
    }

    match op {
        TensorReduceOp::Sum => {
            let mut acc = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc += unsafe { *src.add(i) } as f64;
            }
            acc
        }
        TensorReduceOp::Prod => {
            let mut acc = 1.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc *= unsafe { *src.add(i) } as f64;
            }
            acc
        }
        TensorReduceOp::Max => {
            // SAFETY: n > 0 checked above
            let mut acc = unsafe { *src } as f64;
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc = acc.max(unsafe { *src.add(i) } as f64);
            }
            acc
        }
        TensorReduceOp::Min => {
            // SAFETY: n > 0 checked above
            let mut acc = unsafe { *src } as f64;
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc = acc.min(unsafe { *src.add(i) } as f64);
            }
            acc
        }
        TensorReduceOp::Mean => {
            // SAFETY: Recursive call with same safety guarantees
            let sum = unsafe { reduce_f32(src, n, TensorReduceOp::Sum) };
            sum / n as f64
        }
        TensorReduceOp::All => {
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                if unsafe { *src.add(i) } == 0.0 {
                    return 0.0;
                }
            }
            1.0
        }
        TensorReduceOp::Any => {
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                if unsafe { *src.add(i) } != 0.0 {
                    return 1.0;
                }
            }
            0.0
        }
        TensorReduceOp::Std | TensorReduceOp::Var => {
            // Compute mean first (two-pass algorithm for numerical stability)
            let mut sum = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                sum += unsafe { *src.add(i) } as f64;
            }
            let mean = sum / n as f64;

            // Compute variance: E[(X - mean)^2]
            let mut var_sum = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let diff = unsafe { *src.add(i) } as f64 - mean;
                var_sum += diff * diff;
            }
            let variance = var_sum / n as f64;

            if matches!(op, TensorReduceOp::Std) {
                variance.sqrt()
            } else {
                variance
            }
        }
        TensorReduceOp::Norm => {
            // L2 norm: sqrt(sum(x^2))
            let mut sum_sq = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let v = unsafe { *src.add(i) } as f64;
                sum_sq += v * v;
            }
            sum_sq.sqrt()
        }
        TensorReduceOp::LogSumExp => {
            // Numerically stable log-sum-exp: max + log(sum(exp(x - max)))
            // SAFETY: n > 0 checked above
            let mut max_val = unsafe { *src } as f64;
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let v = unsafe { *src.add(i) } as f64;
                if v > max_val {
                    max_val = v;
                }
            }
            let mut sum_exp = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                sum_exp += (unsafe { *src.add(i) } as f64 - max_val).exp();
            }
            max_val + sum_exp.ln()
        }
    }
}

/// F64 reduction.
unsafe fn reduce_f64(src: *const f64, n: usize, op: TensorReduceOp) -> f64 {
    if n == 0 {
        return 0.0;
    }

    match op {
        TensorReduceOp::Sum => {
            let mut acc = 0.0;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc += unsafe { *src.add(i) };
            }
            acc
        }
        TensorReduceOp::Prod => {
            let mut acc = 1.0;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc *= unsafe { *src.add(i) };
            }
            acc
        }
        TensorReduceOp::Max => {
            // SAFETY: n > 0 checked above
            let mut acc = unsafe { *src };
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc = acc.max(unsafe { *src.add(i) });
            }
            acc
        }
        TensorReduceOp::Min => {
            // SAFETY: n > 0 checked above
            let mut acc = unsafe { *src };
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                acc = acc.min(unsafe { *src.add(i) });
            }
            acc
        }
        TensorReduceOp::Mean => {
            // SAFETY: Recursive call with same safety guarantees
            let sum = unsafe { reduce_f64(src, n, TensorReduceOp::Sum) };
            sum / n as f64
        }
        TensorReduceOp::All => {
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                if unsafe { *src.add(i) } == 0.0 {
                    return 0.0;
                }
            }
            1.0
        }
        TensorReduceOp::Any => {
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                if unsafe { *src.add(i) } != 0.0 {
                    return 1.0;
                }
            }
            0.0
        }
        TensorReduceOp::Std | TensorReduceOp::Var => {
            // Compute mean first (two-pass algorithm for numerical stability)
            let mut sum = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                sum += unsafe { *src.add(i) };
            }
            let mean = sum / n as f64;

            // Compute variance: E[(X - mean)^2]
            let mut var_sum = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let diff = unsafe { *src.add(i) } - mean;
                var_sum += diff * diff;
            }
            let variance = var_sum / n as f64;

            if matches!(op, TensorReduceOp::Std) {
                variance.sqrt()
            } else {
                variance
            }
        }
        TensorReduceOp::Norm => {
            // L2 norm: sqrt(sum(x^2))
            let mut sum_sq = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let v = unsafe { *src.add(i) };
                sum_sq += v * v;
            }
            sum_sq.sqrt()
        }
        TensorReduceOp::LogSumExp => {
            // Numerically stable log-sum-exp: max + log(sum(exp(x - max)))
            // SAFETY: n > 0 checked above
            let mut max_val = unsafe { *src };
            for i in 1..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                let v = unsafe { *src.add(i) };
                if v > max_val {
                    max_val = v;
                }
            }
            let mut sum_exp = 0.0f64;
            for i in 0..n {
                // SAFETY: Caller ensures src points to valid memory of n elements
                sum_exp += (unsafe { *src.add(i) } - max_val).exp();
            }
            max_val + sum_exp.ln()
        }
    }
}

/// Matrix multiplication (2D tensors).
/// Uses kernel dispatch for SIMD (tiled) and GPU (Metal) acceleration.
pub fn tensor_matmul(lhs: &TensorHandle, rhs: &TensorHandle) -> Option<TensorHandle> {
    // Check dimensions: lhs is (M, K), rhs is (K, N), result is (M, N)
    if lhs.ndim != 2 || rhs.ndim != 2 {
        return None;
    }

    let k1 = lhs.shape[1];
    let k2 = rhs.shape[0];

    if k1 != k2 {
        return None;
    }

    if lhs.dtype != rhs.dtype {
        return None;
    }

    // Use kernel dispatch for optimized execution (SIMD/GPU)
    if let Some(result) = super::kernel::dispatch_matmul(lhs, rhs) {
        return Some(result);
    }

    // Fallback to manual implementation
    let m = lhs.shape[0];
    let k = k1;
    let n = rhs.shape[1];

    let result = TensorHandle::zeros(&[m, n], lhs.dtype)?;

    let lhs_data = lhs.data.as_ref()?;
    let rhs_data = rhs.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    unsafe {
        let a = (*lhs_data.as_ptr()).as_ptr();
        let b = (*rhs_data.as_ptr()).as_ptr();
        let c = (*out_data.as_ptr()).as_mut_ptr();

        match lhs.dtype {
            DType::F32 => {
                matmul_f32(a as *const f32, b as *const f32, c as *mut f32, m, k, n);
            }
            DType::F64 => {
                matmul_f64(a as *const f64, b as *const f64, c as *mut f64, m, k, n);
            }
            _ => return None,
        }
    }

    Some(result)
}

/// F32 matrix multiplication (naive implementation).
unsafe fn matmul_f32(a: *const f32, b: *const f32, c: *mut f32, m: usize, k: usize, n: usize) {
    // C[i,j] = sum_l A[i,l] * B[l,j]
    for i in 0..m {
        for j in 0..n {
            let mut sum = 0.0f32;
            for l in 0..k {
                // SAFETY: Caller ensures a and b point to valid matrices of size m*k and k*n
                sum += unsafe { *a.add(i * k + l) * *b.add(l * n + j) };
            }
            // SAFETY: Caller ensures c points to valid matrix of size m*n
            unsafe { *c.add(i * n + j) = sum };
        }
    }
}

/// F64 matrix multiplication (naive implementation).
unsafe fn matmul_f64(a: *const f64, b: *const f64, c: *mut f64, m: usize, k: usize, n: usize) {
    for i in 0..m {
        for j in 0..n {
            let mut sum = 0.0f64;
            for l in 0..k {
                // SAFETY: Caller ensures a and b point to valid matrices of size m*k and k*n
                sum += unsafe { *a.add(i * k + l) * *b.add(l * n + j) };
            }
            // SAFETY: Caller ensures c points to valid matrix of size m*n
            unsafe { *c.add(i * n + j) = sum };
        }
    }
}

/// Reshapes a tensor to a new shape.
pub fn tensor_reshape(src: &TensorHandle, new_shape: &[usize]) -> Option<TensorHandle> {
    // Check that total elements match
    let new_numel: usize = new_shape.iter().product();
    if new_numel != src.numel {
        return None;
    }

    if new_shape.len() > MAX_DIMS {
        return None;
    }

    // Create a view with new shape
    src.incref();

    let mut result = TensorHandle {
        dtype: src.dtype,
        ndim: new_shape.len() as u8,
        flags: (src.flags & !TensorFlags::OWNS_DATA) | TensorFlags::IS_VIEW,
        _reserved: 0,
        numel: new_numel,
        shape: [0; MAX_DIMS],
        strides: [0isize; MAX_DIMS],
        data: src.data,
        offset: src.offset,
    };

    for (i, &dim) in new_shape.iter().enumerate() {
        result.shape[i] = dim;
    }
    result.compute_strides();

    Some(result)
}

/// Transposes a 2D tensor.
pub fn tensor_transpose(src: &TensorHandle) -> Option<TensorHandle> {
    if src.ndim != 2 {
        return None;
    }

    // For transpose, we need to copy data (can't just swap strides for correctness)
    let new_shape = [src.shape[1], src.shape[0]];
    let result = TensorHandle::zeros(&new_shape, src.dtype)?;

    let src_data = src.data.as_ref()?;
    let out_data = result.data.as_ref()?;

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let out_ptr = (*out_data.as_ptr()).as_mut_ptr();

        let rows = src.shape[0];
        let cols = src.shape[1];

        match src.dtype {
            DType::F32 => {
                let s = src_ptr as *const f32;
                let d = out_ptr as *mut f32;
                for i in 0..rows {
                    for j in 0..cols {
                        *d.add(j * rows + i) = *s.add(i * cols + j);
                    }
                }
            }
            DType::F64 => {
                let s = src_ptr as *const f64;
                let d = out_ptr as *mut f64;
                for i in 0..rows {
                    for j in 0..cols {
                        *d.add(j * rows + i) = *s.add(i * cols + j);
                    }
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Clones a tensor (deep copy).
pub fn tensor_clone(src: &TensorHandle) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(&src.shape[..src.ndim as usize], src.dtype)?;

    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: Both pointers are valid and have the same size
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();
        std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, src.numel * src.dtype.size());
    }

    Some(result)
}

/// Creates a tensor with values from start to end (exclusive).
pub fn tensor_arange(start: f64, end: f64, step: f64, dtype: DType) -> Option<TensorHandle> {
    if step == 0.0 {
        return None;
    }

    let n = ((end - start) / step).ceil() as usize;
    if n == 0 {
        return None;
    }

    let result = TensorHandle::zeros(&[n], dtype)?;
    let data = result.data.as_ref()?;

    // SAFETY: data pointer is valid for n elements
    unsafe {
        let ptr = (*data.as_ptr()).as_mut_ptr();
        match dtype {
            DType::F32 => {
                let d = ptr as *mut f32;
                for i in 0..n {
                    *d.add(i) = (start + step * i as f64) as f32;
                }
            }
            DType::F64 => {
                let d = ptr as *mut f64;
                for i in 0..n {
                    *d.add(i) = start + step * i as f64;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Creates a tensor with linearly spaced values.
pub fn tensor_linspace(start: f64, end: f64, steps: usize, dtype: DType) -> Option<TensorHandle> {
    if steps == 0 {
        return None;
    }

    let result = TensorHandle::zeros(&[steps], dtype)?;
    let data = result.data.as_ref()?;

    let step = if steps == 1 {
        0.0
    } else {
        (end - start) / (steps - 1) as f64
    };

    // SAFETY: data pointer is valid for steps elements
    unsafe {
        let ptr = (*data.as_ptr()).as_mut_ptr();
        match dtype {
            DType::F32 => {
                let d = ptr as *mut f32;
                for i in 0..steps {
                    *d.add(i) = (start + step * i as f64) as f32;
                }
            }
            DType::F64 => {
                let d = ptr as *mut f64;
                for i in 0..steps {
                    *d.add(i) = start + step * i as f64;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Concatenates tensors along an axis.
pub fn tensor_concat(tensors: &[&TensorHandle], axis: usize) -> Option<TensorHandle> {
    if tensors.is_empty() {
        return None;
    }

    let first = tensors[0];
    if axis >= first.ndim as usize {
        return None;
    }

    // Check all tensors have same dtype and compatible shapes
    let dtype = first.dtype;
    for t in tensors.iter().skip(1) {
        if t.dtype != dtype || t.ndim != first.ndim {
            return None;
        }
        for i in 0..first.ndim as usize {
            if i != axis && t.shape[i] != first.shape[i] {
                return None;
            }
        }
    }

    // Compute output shape
    let mut out_shape = first.shape;
    out_shape[axis] = tensors.iter().map(|t| t.shape[axis]).sum();

    let result = TensorHandle::zeros(&out_shape[..first.ndim as usize], dtype)?;

    // Copy data
    let dst_data = result.data.as_ref()?;
    let dst_ptr = unsafe { (*dst_data.as_ptr()).as_mut_ptr() };

    let mut offset = 0usize;
    for t in tensors {
        if let Some(src_data) = &t.data {
            // SAFETY: All pointers are valid
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr();
                let bytes = t.numel * dtype.size();
                std::ptr::copy_nonoverlapping(src_ptr, dst_ptr.add(offset), bytes);
                offset += bytes;
            }
        }
    }

    Some(result)
}

/// Computes softmax along an axis (default: last axis).
///
/// Uses GPU acceleration on macOS with Metal for large tensors.
pub fn tensor_softmax(src: &TensorHandle, axis: Option<i32>) -> Option<TensorHandle> {
    // Use kernel dispatch layer for automatic GPU/CPU selection
    super::kernel::dispatch_softmax(src, axis)
}

/// CPU implementation of softmax (used as fallback)
pub fn tensor_softmax_cpu(src: &TensorHandle, _axis: Option<i32>) -> Option<TensorHandle> {
    // Simplified implementation: softmax over all elements
    // Full implementation would handle axis-specific softmax
    let result = TensorHandle::zeros(&src.shape[..src.ndim as usize], src.dtype)?;

    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: All pointers are valid
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        match src.dtype {
            DType::F32 => {
                let s = src_ptr as *const f32;
                let d = dst_ptr as *mut f32;

                // Find max for numerical stability
                let mut max_val = f32::NEG_INFINITY;
                for i in 0..src.numel {
                    max_val = max_val.max(*s.add(i));
                }

                // Compute exp(x - max) and sum
                let mut sum = 0.0f32;
                for i in 0..src.numel {
                    let exp_val = (*s.add(i) - max_val).exp();
                    *d.add(i) = exp_val;
                    sum += exp_val;
                }

                // Normalize
                for i in 0..src.numel {
                    *d.add(i) /= sum;
                }
            }
            DType::F64 => {
                let s = src_ptr as *const f64;
                let d = dst_ptr as *mut f64;

                // Find max for numerical stability
                let mut max_val = f64::NEG_INFINITY;
                for i in 0..src.numel {
                    max_val = max_val.max(*s.add(i));
                }

                // Compute exp(x - max) and sum
                let mut sum = 0.0f64;
                for i in 0..src.numel {
                    let exp_val = (*s.add(i) - max_val).exp();
                    *d.add(i) = exp_val;
                    sum += exp_val;
                }

                // Normalize
                for i in 0..src.numel {
                    *d.add(i) /= sum;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Returns the index of the maximum value along an axis.
pub fn tensor_argmax(src: &TensorHandle, _axis: Option<i32>) -> Option<(usize, f64)> {
    // Simplified implementation: argmax over all elements
    let src_data = src.data.as_ref()?;

    // SAFETY: src_ptr is valid for numel elements
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();

        match src.dtype {
            DType::F32 => {
                let s = src_ptr as *const f32;
                let mut max_idx = 0usize;
                let mut max_val = *s;
                for i in 1..src.numel {
                    if *s.add(i) > max_val {
                        max_val = *s.add(i);
                        max_idx = i;
                    }
                }
                Some((max_idx, max_val as f64))
            }
            DType::F64 => {
                let s = src_ptr as *const f64;
                let mut max_idx = 0usize;
                let mut max_val = *s;
                for i in 1..src.numel {
                    if *s.add(i) > max_val {
                        max_val = *s.add(i);
                        max_idx = i;
                    }
                }
                Some((max_idx, max_val))
            }
            _ => None,
        }
    }
}

/// Creates an identity matrix.
pub fn tensor_identity(n: usize, dtype: DType) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(&[n, n], dtype)?;
    let data = result.data.as_ref()?;

    // SAFETY: data pointer is valid for n*n elements
    unsafe {
        let ptr = (*data.as_ptr()).as_mut_ptr();
        match dtype {
            DType::F32 => {
                let d = ptr as *mut f32;
                for i in 0..n {
                    *d.add(i * n + i) = 1.0;
                }
            }
            DType::F64 => {
                let d = ptr as *mut f64;
                for i in 0..n {
                    *d.add(i * n + i) = 1.0;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Squeezes dimensions of size 1 from a tensor.
pub fn tensor_squeeze(src: &TensorHandle, dim: Option<usize>) -> Option<TensorHandle> {
    let mut new_shape = Vec::new();

    for i in 0..src.ndim as usize {
        if src.shape[i] != 1 {
            new_shape.push(src.shape[i]);
        } else if let Some(d) = dim
            && i != d {
                new_shape.push(src.shape[i]);
            }
    }

    if new_shape.is_empty() {
        new_shape.push(1); // Scalar case
    }

    tensor_reshape(src, &new_shape)
}

/// Creates a tensor from a flat slice of values.
pub fn tensor_from_slice(data: &[f64], shape: &[usize], dtype: DType) -> Option<TensorHandle> {
    let expected_numel: usize = shape.iter().product();
    if data.len() != expected_numel {
        return None;
    }

    let result = TensorHandle::zeros(shape, dtype)?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: dst_ptr is valid for expected_numel elements
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();
        match dtype {
            DType::F32 => {
                let d = dst_ptr as *mut f32;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as f32;
                }
            }
            DType::F64 => {
                let d = dst_ptr as *mut f64;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val;
                }
            }
            DType::I32 => {
                let d = dst_ptr as *mut i32;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as i32;
                }
            }
            DType::I64 => {
                let d = dst_ptr as *mut i64;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as i64;
                }
            }
            DType::U8 => {
                for (i, &val) in data.iter().enumerate() {
                    *dst_ptr.add(i) = val as u8;
                }
            }
            DType::U16 => {
                let d = dst_ptr as *mut u16;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as u16;
                }
            }
            DType::U32 => {
                let d = dst_ptr as *mut u32;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as u32;
                }
            }
            DType::U64 => {
                let d = dst_ptr as *mut u64;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as u64;
                }
            }
            DType::I8 => {
                let d = dst_ptr as *mut i8;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as i8;
                }
            }
            DType::I16 => {
                let d = dst_ptr as *mut i16;
                for (i, &val) in data.iter().enumerate() {
                    *d.add(i) = val as i16;
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Creates a Complex64 tensor from pairs of (real, imag) values.
///
/// Each pair of values in data represents one complex number: [re0, im0, re1, im1, ...]
/// The shape refers to the number of complex elements, not the underlying floats.
pub fn tensor_from_complex64_slice(data: &[f32], shape: &[usize]) -> Option<TensorHandle> {
    let expected_numel: usize = shape.iter().product();
    // data length should be 2x numel (each complex = 2 floats)
    if data.len() != expected_numel * 2 {
        return None;
    }

    let result = TensorHandle::zeros(shape, DType::Complex64)?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: dst_ptr is valid for expected_numel * 2 f32 elements
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;
        for (i, &val) in data.iter().enumerate() {
            *dst_ptr.add(i) = val;
        }
    }

    Some(result)
}

/// Creates a Complex128 tensor from pairs of (real, imag) values.
///
/// Each pair of values in data represents one complex number: [re0, im0, re1, im1, ...]
/// The shape refers to the number of complex elements, not the underlying doubles.
pub fn tensor_from_complex128_slice(data: &[f64], shape: &[usize]) -> Option<TensorHandle> {
    let expected_numel: usize = shape.iter().product();
    // data length should be 2x numel (each complex = 2 doubles)
    if data.len() != expected_numel * 2 {
        return None;
    }

    let result = TensorHandle::zeros(shape, DType::Complex128)?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: dst_ptr is valid for expected_numel * 2 f64 elements
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;
        for (i, &val) in data.iter().enumerate() {
            *dst_ptr.add(i) = val;
        }
    }

    Some(result)
}

/// Creates an F16 tensor from f32 values (which are converted to F16).
///
/// The values are first converted from f32 to F16 format.
pub fn tensor_from_f16_slice(data: &[f32], shape: &[usize]) -> Option<TensorHandle> {
    use crate::interpreter::kernel::cpu::f32_to_f16;

    let expected_numel: usize = shape.iter().product();
    if data.len() != expected_numel {
        return None;
    }

    let result = TensorHandle::zeros(shape, DType::F16)?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: dst_ptr is valid for expected_numel u16 elements
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut u16;
        for (i, &val) in data.iter().enumerate() {
            *dst_ptr.add(i) = f32_to_f16(val);
        }
    }

    Some(result)
}

/// Creates a BF16 tensor from f32 values (which are converted to BF16).
///
/// The values are first converted from f32 to BF16 format.
pub fn tensor_from_bf16_slice(data: &[f32], shape: &[usize]) -> Option<TensorHandle> {
    use crate::interpreter::kernel::cpu::f32_to_bf16;

    let expected_numel: usize = shape.iter().product();
    if data.len() != expected_numel {
        return None;
    }

    let result = TensorHandle::zeros(shape, DType::BF16)?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: dst_ptr is valid for expected_numel u16 elements
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut u16;
        for (i, &val) in data.iter().enumerate() {
            *dst_ptr.add(i) = f32_to_bf16(val);
        }
    }

    Some(result)
}

/// Creates a tensor filled with random values [0, 1).
pub fn tensor_rand(shape: &[usize], dtype: DType) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(shape, dtype)?;
    let data = result.data.as_ref()?;

    // Simple xorshift PRNG for reproducibility
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0x12345678_9ABCDEF0);

    fn next_rand() -> f64 {
        let mut s = SEED.load(Ordering::Relaxed);
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        SEED.store(s, Ordering::Relaxed);
        (s as f64) / (u64::MAX as f64)
    }

    // SAFETY: data ptr is valid for numel elements
    unsafe {
        let ptr = (*data.as_ptr()).as_mut_ptr();
        match dtype {
            DType::F32 => {
                let d = ptr as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = next_rand() as f32;
                }
            }
            DType::F64 => {
                let d = ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = next_rand();
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Slices a tensor along specified ranges.
pub fn tensor_slice(src: &TensorHandle, ranges: &[(usize, usize)]) -> Option<TensorHandle> {
    if ranges.len() != src.ndim as usize {
        return None;
    }

    // Validate ranges
    let mut new_shape = Vec::with_capacity(ranges.len());
    for (i, &(start, end)) in ranges.iter().enumerate() {
        if start >= end || end > src.shape[i] {
            return None;
        }
        new_shape.push(end - start);
    }

    let result = TensorHandle::zeros(&new_shape, src.dtype)?;
    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // Copy sliced data
    // For simplicity, handle contiguous 1D and 2D cases
    if src.ndim == 1 {
        let (start, _end) = ranges[0];
        // SAFETY: pointers are valid for the specified ranges
        unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr();
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();
            let bytes = result.numel * src.dtype.size();
            std::ptr::copy_nonoverlapping(
                src_ptr.add(start * src.dtype.size()),
                dst_ptr,
                bytes,
            );
        }
    } else if src.ndim == 2 {
        let (row_start, _row_end) = ranges[0];
        let (col_start, _col_end) = ranges[1];
        let new_rows = new_shape[0];
        let new_cols = new_shape[1];
        let src_cols = src.shape[1];

        // SAFETY: pointers are valid for the specified ranges
        unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr();
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

            match src.dtype {
                DType::F32 => {
                    let s = src_ptr as *const f32;
                    let d = dst_ptr as *mut f32;
                    for i in 0..new_rows {
                        for j in 0..new_cols {
                            *d.add(i * new_cols + j) =
                                *s.add((row_start + i) * src_cols + (col_start + j));
                        }
                    }
                }
                DType::F64 => {
                    let s = src_ptr as *const f64;
                    let d = dst_ptr as *mut f64;
                    for i in 0..new_rows {
                        for j in 0..new_cols {
                            *d.add(i * new_cols + j) =
                                *s.add((row_start + i) * src_cols + (col_start + j));
                        }
                    }
                }
                _ => return None,
            }
        }
    } else {
        // Generic N-dimensional slice - use recursive copy
        // For now, return None for higher dimensions
        return None;
    }

    Some(result)
}

/// Stacks tensors along a new axis.
pub fn tensor_stack(tensors: &[&TensorHandle], axis: usize) -> Option<TensorHandle> {
    if tensors.is_empty() {
        return None;
    }

    let first = tensors[0];

    // All tensors must have same shape and dtype
    for t in tensors.iter().skip(1) {
        if t.dtype != first.dtype || t.ndim != first.ndim {
            return None;
        }
        for i in 0..first.ndim as usize {
            if t.shape[i] != first.shape[i] {
                return None;
            }
        }
    }

    // Build new shape with extra dimension at axis
    if axis > first.ndim as usize {
        return None;
    }

    let mut new_shape = Vec::with_capacity(first.ndim as usize + 1);
    for i in 0..first.ndim as usize {
        if i == axis {
            new_shape.push(tensors.len());
        }
        new_shape.push(first.shape[i]);
    }
    if axis == first.ndim as usize {
        new_shape.push(tensors.len());
    }

    let result = TensorHandle::zeros(&new_shape, first.dtype)?;
    let dst_data = result.data.as_ref()?;

    // Copy each tensor
    let elem_size = first.dtype.size();
    let tensor_size = first.numel * elem_size;

    // SAFETY: all pointers are valid
    unsafe {
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();
        for (i, t) in tensors.iter().enumerate() {
            if let Some(src_data) = &t.data {
                let src_ptr = (*src_data.as_ptr()).as_ptr();
                std::ptr::copy_nonoverlapping(
                    src_ptr,
                    dst_ptr.add(i * tensor_size),
                    tensor_size,
                );
            }
        }
    }

    Some(result)
}

/// Broadcasts a tensor to a new shape.
pub fn tensor_broadcast(src: &TensorHandle, new_shape: &[usize]) -> Option<TensorHandle> {
    // Validate broadcast compatibility
    // Each dimension must either match or be 1 in src
    let src_ndim = src.ndim as usize;
    let new_ndim = new_shape.len();

    if new_ndim < src_ndim {
        return None;
    }

    // Align shapes from the right
    let offset = new_ndim - src_ndim;
    for i in 0..src_ndim {
        let src_dim = src.shape[i];
        let new_dim = new_shape[offset + i];
        if src_dim != 1 && src_dim != new_dim {
            return None;
        }
    }

    let result = TensorHandle::zeros(new_shape, src.dtype)?;
    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: all pointers are valid
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        match src.dtype {
            DType::F32 => {
                broadcast_copy::<f32>(
                    src_ptr as *const f32,
                    dst_ptr as *mut f32,
                    &src.shape[..src_ndim],
                    &src.strides[..src_ndim],
                    new_shape,
                );
            }
            DType::F64 => {
                broadcast_copy::<f64>(
                    src_ptr as *const f64,
                    dst_ptr as *mut f64,
                    &src.shape[..src_ndim],
                    &src.strides[..src_ndim],
                    new_shape,
                );
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Helper to copy with broadcasting.
///
/// # Safety
///
/// - `src` must be valid for reads of `src_shape.iter().product()` elements
/// - `dst` must be valid for writes of `dst_shape.iter().product()` elements
/// - src_strides must correspond to src_shape dimensions
/// - Supports negative strides for reverse iteration
unsafe fn broadcast_copy<T: Copy>(
    src: *const T,
    dst: *mut T,
    src_shape: &[usize],
    src_strides: &[isize],
    dst_shape: &[usize],
) {
    let dst_numel: usize = dst_shape.iter().product();
    let src_ndim = src_shape.len();
    let dst_ndim = dst_shape.len();
    let offset = dst_ndim - src_ndim;

    for linear_idx in 0..dst_numel {
        // Convert linear index to multi-index
        // Use signed arithmetic to support negative strides
        let mut remaining = linear_idx;
        let mut src_idx = 0isize;

        for i in (0..dst_ndim).rev() {
            let coord = remaining % dst_shape[i];
            remaining /= dst_shape[i];

            if i >= offset {
                let src_dim_idx = i - offset;
                if src_dim_idx < src_ndim {
                    // If src dimension is 1, use 0; otherwise use coord
                    let src_coord = if src_shape[src_dim_idx] == 1 {
                        0
                    } else {
                        coord as isize
                    };
                    src_idx += src_coord * src_strides[src_dim_idx];
                }
            }
        }

        // SAFETY: Caller ensures src and dst are valid for the computed indices
        debug_assert!(src_idx >= 0, "Negative source index indicates invalid view");
        unsafe {
            *dst.add(linear_idx) = *src.offset(src_idx);
        }
    }
}

/// Conditional select: where(cond, x, y).
pub fn tensor_where(
    cond: &TensorHandle,
    x: &TensorHandle,
    y: &TensorHandle,
) -> Option<TensorHandle> {
    // All tensors must have same shape
    if cond.ndim != x.ndim || x.ndim != y.ndim {
        return None;
    }
    for i in 0..cond.ndim as usize {
        if cond.shape[i] != x.shape[i] || x.shape[i] != y.shape[i] {
            return None;
        }
    }
    if x.dtype != y.dtype {
        return None;
    }

    let result = TensorHandle::zeros(&x.shape[..x.ndim as usize], x.dtype)?;

    let cond_data = cond.data.as_ref()?;
    let x_data = x.data.as_ref()?;
    let y_data = y.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: all pointers are valid
    unsafe {
        let cond_ptr = (*cond_data.as_ptr()).as_ptr();
        let x_ptr = (*x_data.as_ptr()).as_ptr();
        let y_ptr = (*y_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        match x.dtype {
            DType::F32 => {
                let c = cond_ptr as *const f32;
                let xp = x_ptr as *const f32;
                let yp = y_ptr as *const f32;
                let d = dst_ptr as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = if *c.add(i) != 0.0 {
                        *xp.add(i)
                    } else {
                        *yp.add(i)
                    };
                }
            }
            DType::F64 => {
                let c = cond_ptr as *const f64;
                let xp = x_ptr as *const f64;
                let yp = y_ptr as *const f64;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = if *c.add(i) != 0.0 {
                        *xp.add(i)
                    } else {
                        *yp.add(i)
                    };
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Clamps tensor values to [min, max].
pub fn tensor_clamp(src: &TensorHandle, min: f64, max: f64) -> Option<TensorHandle> {
    let result = TensorHandle::zeros(&src.shape[..src.ndim as usize], src.dtype)?;

    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: all pointers are valid
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        match src.dtype {
            DType::F32 => {
                let s = src_ptr as *const f32;
                let d = dst_ptr as *mut f32;
                let min_f = min as f32;
                let max_f = max as f32;
                for i in 0..result.numel {
                    *d.add(i) = (*s.add(i)).clamp(min_f, max_f);
                }
            }
            DType::F64 => {
                let s = src_ptr as *const f64;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = (*s.add(i)).clamp(min, max);
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

/// Casts a tensor to a different dtype.
pub fn tensor_cast(src: &TensorHandle, dst_dtype: DType) -> Option<TensorHandle> {
    if src.dtype == dst_dtype {
        return tensor_clone(src);
    }

    let result = TensorHandle::zeros(&src.shape[..src.ndim as usize], dst_dtype)?;

    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: all pointers are valid
    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        // Type conversion matrix
        match (src.dtype, dst_dtype) {
            (DType::F32, DType::F64) => {
                let s = src_ptr as *const f32;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as f64;
                }
            }
            (DType::F64, DType::F32) => {
                let s = src_ptr as *const f64;
                let d = dst_ptr as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as f32;
                }
            }
            (DType::I32, DType::F32) => {
                let s = src_ptr as *const i32;
                let d = dst_ptr as *mut f32;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as f32;
                }
            }
            (DType::I32, DType::F64) => {
                let s = src_ptr as *const i32;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as f64;
                }
            }
            (DType::F32, DType::I32) => {
                let s = src_ptr as *const f32;
                let d = dst_ptr as *mut i32;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as i32;
                }
            }
            (DType::F64, DType::I32) => {
                let s = src_ptr as *const f64;
                let d = dst_ptr as *mut i32;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as i32;
                }
            }
            (DType::I64, DType::F64) => {
                let s = src_ptr as *const i64;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as f64;
                }
            }
            (DType::F64, DType::I64) => {
                let s = src_ptr as *const f64;
                let d = dst_ptr as *mut i64;
                for i in 0..result.numel {
                    *d.add(i) = *s.add(i) as i64;
                }
            }
            _ => return None, // Unsupported cast
        }
    }

    Some(result)
}

/// Linear interpolation between two tensors.
pub fn tensor_lerp(a: &TensorHandle, b: &TensorHandle, t: f64) -> Option<TensorHandle> {
    if a.dtype != b.dtype || a.ndim != b.ndim {
        return None;
    }
    for i in 0..a.ndim as usize {
        if a.shape[i] != b.shape[i] {
            return None;
        }
    }

    let result = TensorHandle::zeros(&a.shape[..a.ndim as usize], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    // SAFETY: all pointers are valid
    unsafe {
        let a_ptr = (*a_data.as_ptr()).as_ptr();
        let b_ptr = (*b_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        match a.dtype {
            DType::F32 => {
                let ap = a_ptr as *const f32;
                let bp = b_ptr as *const f32;
                let d = dst_ptr as *mut f32;
                let t_f = t as f32;
                for i in 0..result.numel {
                    let av = *ap.add(i);
                    let bv = *bp.add(i);
                    *d.add(i) = av + t_f * (bv - av);
                }
            }
            DType::F64 => {
                let ap = a_ptr as *const f64;
                let bp = b_ptr as *const f64;
                let d = dst_ptr as *mut f64;
                for i in 0..result.numel {
                    let av = *ap.add(i);
                    let bv = *bp.add(i);
                    *d.add(i) = av + t * (bv - av);
                }
            }
            _ => return None,
        }
    }

    Some(result)
}

// ============================================================================
// Normalization Operations
// ============================================================================

/// Layer normalization across the last axis.
///
/// For input of shape [*, N], normalizes across N:
/// output = (input - mean) / sqrt(var + eps) * gamma + beta
///
/// Uses GPU acceleration on macOS with Metal for large F32 2D tensors.
pub fn tensor_layer_norm(
    input: &TensorHandle,
    gamma: Option<&TensorHandle>, // Scale, shape [N]
    beta: Option<&TensorHandle>,  // Bias, shape [N]
    eps: f64,
) -> Option<TensorHandle> {
    // Use kernel dispatch layer for automatic GPU/CPU selection
    super::kernel::dispatch_layer_norm(input, gamma, beta, eps)
}

/// CPU implementation of layer normalization (used as fallback)
pub fn tensor_layer_norm_cpu(
    input: &TensorHandle,
    gamma: Option<&TensorHandle>, // Scale, shape [N]
    beta: Option<&TensorHandle>,  // Bias, shape [N]
    eps: f64,
) -> Option<TensorHandle> {
    if input.ndim == 0 {
        return None;
    }

    let norm_axis = input.ndim as usize - 1;
    let norm_size = input.shape[norm_axis];

    // Validate gamma/beta shapes if provided
    if let Some(g) = gamma
        && (g.ndim != 1 || g.shape[0] != norm_size) {
            return None;
        }
    if let Some(b) = beta
        && (b.ndim != 1 || b.shape[0] != norm_size) {
            return None;
        }

    let result = TensorHandle::zeros(&input.shape[..input.ndim as usize], input.dtype)?;

    let src_data = input.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    match input.dtype {
        DType::F32 => {
            let eps_f = eps as f32;
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

                let gamma_ptr = gamma.and_then(|g| g.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);
                let beta_ptr = beta.and_then(|b| b.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);

                let batch_size = result.numel / norm_size;

                for batch in 0..batch_size {
                    let offset = batch * norm_size;

                    // Compute mean
                    let mut sum = 0.0f32;
                    for i in 0..norm_size {
                        sum += *src_ptr.add(offset + i);
                    }
                    let mean = sum / norm_size as f32;

                    // Compute variance
                    let mut var = 0.0f32;
                    for i in 0..norm_size {
                        let diff = *src_ptr.add(offset + i) - mean;
                        var += diff * diff;
                    }
                    var /= norm_size as f32;

                    // Normalize and apply gamma/beta
                    let inv_std = 1.0 / (var + eps_f).sqrt();
                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        let mut y = (x - mean) * inv_std;

                        if let Some(gp) = gamma_ptr {
                            y *= *gp.add(i);
                        }
                        if let Some(bp) = beta_ptr {
                            y += *bp.add(i);
                        }

                        *dst_ptr.add(offset + i) = y;
                    }
                }
            }
        }
        DType::F64 => {
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

                let gamma_ptr = gamma.and_then(|g| g.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f64);
                let beta_ptr = beta.and_then(|b| b.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f64);

                let batch_size = result.numel / norm_size;

                for batch in 0..batch_size {
                    let offset = batch * norm_size;

                    let mut sum = 0.0f64;
                    for i in 0..norm_size {
                        sum += *src_ptr.add(offset + i);
                    }
                    let mean = sum / norm_size as f64;

                    let mut var = 0.0f64;
                    for i in 0..norm_size {
                        let diff = *src_ptr.add(offset + i) - mean;
                        var += diff * diff;
                    }
                    var /= norm_size as f64;

                    let inv_std = 1.0 / (var + eps).sqrt();
                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        let mut y = (x - mean) * inv_std;

                        if let Some(gp) = gamma_ptr {
                            y *= *gp.add(i);
                        }
                        if let Some(bp) = beta_ptr {
                            y += *bp.add(i);
                        }

                        *dst_ptr.add(offset + i) = y;
                    }
                }
            }
        }
        _ => return None,
    }

    Some(result)
}

/// Batch normalization (typically for [N, C, H, W] tensors).
///
/// Normalizes across N, H, W for each channel C.
pub fn tensor_batch_norm(
    input: &TensorHandle,           // [N, C, H, W] or [N, C]
    gamma: Option<&TensorHandle>,   // [C]
    beta: Option<&TensorHandle>,    // [C]
    running_mean: Option<&TensorHandle>, // [C]
    running_var: Option<&TensorHandle>,  // [C]
    eps: f64,
    training: bool,
) -> Option<TensorHandle> {
    // For inference, use running statistics
    // For training, compute batch statistics (simplified - just do inference for now)
    if input.ndim < 2 {
        return None;
    }

    let channels = input.shape[1];

    // Validate shapes
    if let Some(g) = gamma
        && (g.ndim != 1 || g.shape[0] != channels) {
            return None;
        }

    let result = TensorHandle::zeros(&input.shape[..input.ndim as usize], input.dtype)?;

    let src_data = input.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    match input.dtype {
        DType::F32 => {
            let eps_f = eps as f32;
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

                let gamma_ptr = gamma.and_then(|g| g.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);
                let beta_ptr = beta.and_then(|b| b.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);
                let mean_ptr = running_mean.and_then(|m| m.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);
                let var_ptr = running_var.and_then(|v| v.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);

                let batch_size = input.shape[0];
                let spatial_size: usize = if input.ndim > 2 {
                    input.shape[2..input.ndim as usize].iter().product()
                } else {
                    1
                };

                if !training {
                    // Inference mode: use running statistics
                    for n in 0..batch_size {
                        for c in 0..channels {
                            let mean = mean_ptr.map(|p| *p.add(c)).unwrap_or(0.0);
                            let var = var_ptr.map(|p| *p.add(c)).unwrap_or(1.0);
                            let g = gamma_ptr.map(|p| *p.add(c)).unwrap_or(1.0);
                            let b = beta_ptr.map(|p| *p.add(c)).unwrap_or(0.0);

                            let inv_std = 1.0 / (var + eps_f).sqrt();

                            for s in 0..spatial_size {
                                let idx = n * channels * spatial_size + c * spatial_size + s;
                                let x = *src_ptr.add(idx);
                                *dst_ptr.add(idx) = g * (x - mean) * inv_std + b;
                            }
                        }
                    }
                } else {
                    // Training mode: compute batch statistics
                    for c in 0..channels {
                        // Compute mean across batch and spatial dims
                        let mut sum = 0.0f32;
                        let count = batch_size * spatial_size;

                        for n in 0..batch_size {
                            for s in 0..spatial_size {
                                let idx = n * channels * spatial_size + c * spatial_size + s;
                                sum += *src_ptr.add(idx);
                            }
                        }
                        let mean = sum / count as f32;

                        // Compute variance
                        let mut var = 0.0f32;
                        for n in 0..batch_size {
                            for s in 0..spatial_size {
                                let idx = n * channels * spatial_size + c * spatial_size + s;
                                let diff = *src_ptr.add(idx) - mean;
                                var += diff * diff;
                            }
                        }
                        var /= count as f32;

                        let g = gamma_ptr.map(|p| *p.add(c)).unwrap_or(1.0);
                        let b = beta_ptr.map(|p| *p.add(c)).unwrap_or(0.0);
                        let inv_std = 1.0 / (var + eps_f).sqrt();

                        // Normalize
                        for n in 0..batch_size {
                            for s in 0..spatial_size {
                                let idx = n * channels * spatial_size + c * spatial_size + s;
                                let x = *src_ptr.add(idx);
                                *dst_ptr.add(idx) = g * (x - mean) * inv_std + b;
                            }
                        }
                    }
                }
            }
        }
        _ => return None, // Only F32 for now
    }

    Some(result)
}

/// RMS normalization (used in modern LLMs like Grok, LLaMA).
///
/// For input of shape [*, N], normalizes using RMS:
/// output = input / sqrt(mean(x^2) + eps) * gamma
pub fn tensor_rms_norm(
    input: &TensorHandle,
    gamma: Option<&TensorHandle>, // Scale, shape [N]
    eps: f64,
) -> Option<TensorHandle> {
    if input.ndim == 0 {
        return None;
    }

    let norm_axis = input.ndim as usize - 1;
    let norm_size = input.shape[norm_axis];

    // Validate gamma shape if provided
    if let Some(g) = gamma
        && (g.ndim != 1 || g.shape[0] != norm_size) {
            return None;
        }

    let result = TensorHandle::zeros(&input.shape[..input.ndim as usize], input.dtype)?;

    let src_data = input.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    match input.dtype {
        DType::F32 => {
            let eps_f = eps as f32;
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

                let gamma_ptr = gamma.and_then(|g| g.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f32);

                let batch_size = result.numel / norm_size;

                for batch in 0..batch_size {
                    let offset = batch * norm_size;

                    // Compute RMS = sqrt(mean(x^2))
                    let mut sum_sq = 0.0f32;
                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        sum_sq += x * x;
                    }
                    let rms = (sum_sq / norm_size as f32 + eps_f).sqrt();
                    let inv_rms = 1.0 / rms;

                    // Normalize and apply gamma
                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        let mut y = x * inv_rms;

                        if let Some(gp) = gamma_ptr {
                            y *= *gp.add(i);
                        }

                        *dst_ptr.add(offset + i) = y;
                    }
                }
            }
        }
        DType::F64 => {
            unsafe {
                let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

                let gamma_ptr = gamma.and_then(|g| g.data.as_ref())
                    .map(|d| (*d.as_ptr()).as_ptr() as *const f64);

                let batch_size = result.numel / norm_size;

                for batch in 0..batch_size {
                    let offset = batch * norm_size;

                    let mut sum_sq = 0.0f64;
                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        sum_sq += x * x;
                    }
                    let rms = (sum_sq / norm_size as f64 + eps).sqrt();
                    let inv_rms = 1.0 / rms;

                    for i in 0..norm_size {
                        let x = *src_ptr.add(offset + i);
                        let mut y = x * inv_rms;

                        if let Some(gp) = gamma_ptr {
                            y *= *gp.add(i);
                        }

                        *dst_ptr.add(offset + i) = y;
                    }
                }
            }
        }
        _ => return None,
    }

    Some(result)
}

// ============================================================================
// Advanced Tensor Operations
// ============================================================================

/// Dot product of two tensors.
///
/// For 1D tensors: returns scalar (sum of element-wise products)
/// For 2D tensors: matrix multiplication
/// For higher dims: sum over last axis of a and second-to-last of b
pub fn tensor_dot(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    // Handle 1D dot product (inner product)
    if a.ndim == 1 && b.ndim == 1 {
        if a.shape[0] != b.shape[0] {
            return None;
        }

        let result = TensorHandle::zeros(&[], a.dtype)?;

        let a_data = a.data.as_ref()?;
        let b_data = b.data.as_ref()?;
        let dst_data = result.data.as_ref()?;

        match a.dtype {
            DType::F32 => unsafe {
                let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f32;
                let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f32;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

                let mut sum = 0.0f32;
                for i in 0..a.shape[0] {
                    sum += *a_ptr.add(i) * *b_ptr.add(i);
                }
                *dst_ptr = sum;
            },
            DType::F64 => unsafe {
                let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f64;
                let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f64;
                let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

                let mut sum = 0.0f64;
                for i in 0..a.shape[0] {
                    sum += *a_ptr.add(i) * *b_ptr.add(i);
                }
                *dst_ptr = sum;
            },
            _ => return None,
        }

        return Some(result);
    }

    // For 2D tensors, use matmul
    if a.ndim == 2 && b.ndim == 2 {
        return tensor_matmul(a, b);
    }

    // General case: contract last axis of a with second-to-last of b
    // This is simplified - full implementation would be more complex
    None
}

/// Outer product of two tensors.
///
/// For vectors a[n] and b[m], returns a[n, m] where result[i,j] = a[i] * b[j]
pub fn tensor_outer(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    // Both must be 1D
    if a.ndim != 1 || b.ndim != 1 {
        return None;
    }

    let n = a.shape[0];
    let m = b.shape[0];
    let result = TensorHandle::zeros(&[n, m], a.dtype)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    match a.dtype {
        DType::F32 => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f32;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

            for i in 0..n {
                for j in 0..m {
                    *dst_ptr.add(i * m + j) = *a_ptr.add(i) * *b_ptr.add(j);
                }
            }
        },
        DType::F64 => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f64;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

            for i in 0..n {
                for j in 0..m {
                    *dst_ptr.add(i * m + j) = *a_ptr.add(i) * *b_ptr.add(j);
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Batched matrix multiplication.
///
/// For tensors a[..., m, k] and b[..., k, n], returns [..., m, n]
/// Batch dimensions are broadcast.
pub fn tensor_batch_matmul(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.ndim < 2 || b.ndim < 2 {
        return None;
    }

    let a_ndim = a.ndim as usize;
    let b_ndim = b.ndim as usize;

    let m = a.shape[a_ndim - 2];
    let k1 = a.shape[a_ndim - 1];
    let k2 = b.shape[b_ndim - 2];
    let n = b.shape[b_ndim - 1];

    if k1 != k2 {
        return None;
    }
    let k = k1;

    // Compute batch dimensions
    let a_batch: Vec<usize> = a.shape[..a_ndim - 2].to_vec();
    let b_batch: Vec<usize> = b.shape[..b_ndim - 2].to_vec();

    // Broadcast batch dimensions
    let max_batch_ndim = a_batch.len().max(b_batch.len());
    let mut out_batch = Vec::with_capacity(max_batch_ndim);

    for i in 0..max_batch_ndim {
        let a_dim = if i < max_batch_ndim - a_batch.len() {
            1
        } else {
            a_batch[i - (max_batch_ndim - a_batch.len())]
        };
        let b_dim = if i < max_batch_ndim - b_batch.len() {
            1
        } else {
            b_batch[i - (max_batch_ndim - b_batch.len())]
        };

        if a_dim == b_dim {
            out_batch.push(a_dim);
        } else if a_dim == 1 {
            out_batch.push(b_dim);
        } else if b_dim == 1 {
            out_batch.push(a_dim);
        } else {
            return None; // Incompatible batch dimensions
        }
    }

    // Build output shape
    let mut out_shape = out_batch.clone();
    out_shape.push(m);
    out_shape.push(n);

    let result = TensorHandle::zeros(&out_shape, a.dtype)?;
    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let batch_size: usize = out_batch.iter().product();

    match a.dtype {
        DType::F32 => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f32;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

            for batch in 0..batch_size {
                let a_offset = batch * m * k;
                let b_offset = batch * k * n;
                let dst_offset = batch * m * n;

                for i in 0..m {
                    for j in 0..n {
                        let mut sum = 0.0f32;
                        for kk in 0..k {
                            sum += *a_ptr.add(a_offset + i * k + kk)
                                * *b_ptr.add(b_offset + kk * n + j);
                        }
                        *dst_ptr.add(dst_offset + i * n + j) = sum;
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f64;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

            for batch in 0..batch_size {
                let a_offset = batch * m * k;
                let b_offset = batch * k * n;
                let dst_offset = batch * m * n;

                for i in 0..m {
                    for j in 0..n {
                        let mut sum = 0.0f64;
                        for kk in 0..k {
                            sum += *a_ptr.add(a_offset + i * k + kk)
                                * *b_ptr.add(b_offset + kk * n + j);
                        }
                        *dst_ptr.add(dst_offset + i * n + j) = sum;
                    }
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Top-K values and indices.
///
/// Returns (values, indices) for the k largest elements along the specified axis.
pub fn tensor_topk(
    src: &TensorHandle,
    k: usize,
    axis: Option<i32>,
    largest: bool,
    sorted: bool,
) -> Option<(TensorHandle, TensorHandle)> {
    if src.ndim == 0 || k == 0 {
        return None;
    }

    let ndim = src.ndim as usize;
    let axis = match axis {
        Some(a) if a >= 0 => a as usize,
        Some(a) => (ndim as i32 + a) as usize,
        None => ndim - 1, // Default to last axis
    };

    if axis >= ndim {
        return None;
    }

    let axis_size = src.shape[axis];
    if k > axis_size {
        return None;
    }

    // Build output shape
    let mut out_shape = src.shape[..ndim].to_vec();
    out_shape[axis] = k;

    let values = TensorHandle::zeros(&out_shape, src.dtype)?;
    let indices = TensorHandle::zeros(&out_shape, DType::I64)?;

    let src_data = src.data.as_ref()?;
    let val_data = values.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;

    // Calculate strides for iteration
    let outer_size: usize = src.shape[..axis].iter().product();
    let inner_size: usize = src.shape[axis + 1..ndim].iter().product();

    match src.dtype {
        DType::F32 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let val_ptr = (*val_data.as_ptr()).as_mut_ptr() as *mut f32;
            let idx_ptr = (*idx_data.as_ptr()).as_mut_ptr() as *mut i64;

            for outer in 0..outer_size {
                for inner in 0..inner_size {
                    // Collect values along axis
                    let mut pairs: Vec<(f32, usize)> = Vec::with_capacity(axis_size);
                    for i in 0..axis_size {
                        let src_idx = outer * axis_size * inner_size + i * inner_size + inner;
                        pairs.push((*src_ptr.add(src_idx), i));
                    }

                    // Sort by value
                    if largest {
                        pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    } else {
                        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    }

                    if !sorted {
                        // Keep original order for top-k elements (sort by index)
                        pairs.truncate(k);
                        pairs.sort_by_key(|p| p.1);
                    }

                    // Write top-k to output
                    for (i, (val, idx)) in pairs.iter().take(k).enumerate() {
                        let out_idx = outer * k * inner_size + i * inner_size + inner;
                        *val_ptr.add(out_idx) = *val;
                        *idx_ptr.add(out_idx) = *idx as i64;
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let val_ptr = (*val_data.as_ptr()).as_mut_ptr() as *mut f64;
            let idx_ptr = (*idx_data.as_ptr()).as_mut_ptr() as *mut i64;

            for outer in 0..outer_size {
                for inner in 0..inner_size {
                    let mut pairs: Vec<(f64, usize)> = Vec::with_capacity(axis_size);
                    for i in 0..axis_size {
                        let src_idx = outer * axis_size * inner_size + i * inner_size + inner;
                        pairs.push((*src_ptr.add(src_idx), i));
                    }

                    if largest {
                        pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                    } else {
                        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    }

                    if !sorted {
                        pairs.truncate(k);
                        pairs.sort_by_key(|p| p.1);
                    }

                    for (i, (val, idx)) in pairs.iter().take(k).enumerate() {
                        let out_idx = outer * k * inner_size + i * inner_size + inner;
                        *val_ptr.add(out_idx) = *val;
                        *idx_ptr.add(out_idx) = *idx as i64;
                    }
                }
            }
        },
        _ => return None,
    }

    Some((values, indices))
}

/// Cumulative operation type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CumulativeOp {
    Sum,
    Prod,
    Max,
    Min,
}

/// Cumulative sum/product/max/min along an axis.
pub fn tensor_cumulative(
    src: &TensorHandle,
    op: CumulativeOp,
    axis: Option<i32>,
) -> Option<TensorHandle> {
    if src.ndim == 0 {
        return tensor_clone(src);
    }

    let ndim = src.ndim as usize;
    let axis = match axis {
        Some(a) if a >= 0 => a as usize,
        Some(a) => (ndim as i32 + a) as usize,
        None => ndim - 1,
    };

    if axis >= ndim {
        return None;
    }

    let result = TensorHandle::zeros(&src.shape[..ndim], src.dtype)?;

    let src_data = src.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let axis_size = src.shape[axis];
    let outer_size: usize = src.shape[..axis].iter().product();
    let inner_size: usize = src.shape[axis + 1..ndim].iter().product();

    match src.dtype {
        DType::F32 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

            for outer in 0..outer_size {
                for inner in 0..inner_size {
                    let mut acc = match op {
                        CumulativeOp::Sum => 0.0f32,
                        CumulativeOp::Prod => 1.0f32,
                        CumulativeOp::Max => f32::NEG_INFINITY,
                        CumulativeOp::Min => f32::INFINITY,
                    };

                    for i in 0..axis_size {
                        let idx = outer * axis_size * inner_size + i * inner_size + inner;
                        let val = *src_ptr.add(idx);

                        acc = match op {
                            CumulativeOp::Sum => acc + val,
                            CumulativeOp::Prod => acc * val,
                            CumulativeOp::Max => acc.max(val),
                            CumulativeOp::Min => acc.min(val),
                        };

                        *dst_ptr.add(idx) = acc;
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

            for outer in 0..outer_size {
                for inner in 0..inner_size {
                    let mut acc = match op {
                        CumulativeOp::Sum => 0.0f64,
                        CumulativeOp::Prod => 1.0f64,
                        CumulativeOp::Max => f64::NEG_INFINITY,
                        CumulativeOp::Min => f64::INFINITY,
                    };

                    for i in 0..axis_size {
                        let idx = outer * axis_size * inner_size + i * inner_size + inner;
                        let val = *src_ptr.add(idx);

                        acc = match op {
                            CumulativeOp::Sum => acc + val,
                            CumulativeOp::Prod => acc * val,
                            CumulativeOp::Max => acc.max(val),
                            CumulativeOp::Min => acc.min(val),
                        };

                        *dst_ptr.add(idx) = acc;
                    }
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Pooling operation type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PoolOp {
    Max,
    Avg,
    Sum,
}

/// 2D pooling operation.
///
/// Input: [N, C, H, W]
/// Output: [N, C, H_out, W_out]
pub fn tensor_pool2d(
    input: &TensorHandle,
    op: PoolOp,
    kernel_size: (usize, usize),
    stride: (usize, usize),
    padding: (usize, usize),
) -> Option<TensorHandle> {
    if input.ndim != 4 {
        return None;
    }

    let (n, c, h, w) = (
        input.shape[0],
        input.shape[1],
        input.shape[2],
        input.shape[3],
    );
    let (kh, kw) = kernel_size;
    let (sh, sw) = stride;
    let (ph, pw) = padding;

    // Calculate output dimensions
    let h_out = (h + 2 * ph - kh) / sh + 1;
    let w_out = (w + 2 * pw - kw) / sw + 1;

    let result = TensorHandle::zeros(&[n, c, h_out, w_out], input.dtype)?;

    let src_data = input.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    match input.dtype {
        DType::F32 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

            for batch in 0..n {
                for channel in 0..c {
                    for oh in 0..h_out {
                        for ow in 0..w_out {
                            let h_start = (oh * sh).saturating_sub(ph);
                            let w_start = (ow * sw).saturating_sub(pw);
                            let h_end = (h_start + kh).min(h);
                            let w_end = (w_start + kw).min(w);

                            let mut pool_val = match op {
                                PoolOp::Max => f32::NEG_INFINITY,
                                PoolOp::Avg | PoolOp::Sum => 0.0f32,
                            };
                            let mut count = 0;

                            for ih in h_start..h_end {
                                for iw in w_start..w_end {
                                    let src_idx = batch * c * h * w
                                        + channel * h * w
                                        + ih * w
                                        + iw;
                                    let val = *src_ptr.add(src_idx);

                                    pool_val = match op {
                                        PoolOp::Max => pool_val.max(val),
                                        PoolOp::Avg | PoolOp::Sum => pool_val + val,
                                    };
                                    count += 1;
                                }
                            }

                            if op == PoolOp::Avg && count > 0 {
                                pool_val /= count as f32;
                            }

                            let dst_idx = batch * c * h_out * w_out
                                + channel * h_out * w_out
                                + oh * w_out
                                + ow;
                            *dst_ptr.add(dst_idx) = pool_val;
                        }
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

            for batch in 0..n {
                for channel in 0..c {
                    for oh in 0..h_out {
                        for ow in 0..w_out {
                            let h_start = (oh * sh).saturating_sub(ph);
                            let w_start = (ow * sw).saturating_sub(pw);
                            let h_end = (h_start + kh).min(h);
                            let w_end = (w_start + kw).min(w);

                            let mut pool_val = match op {
                                PoolOp::Max => f64::NEG_INFINITY,
                                PoolOp::Avg | PoolOp::Sum => 0.0f64,
                            };
                            let mut count = 0;

                            for ih in h_start..h_end {
                                for iw in w_start..w_end {
                                    let src_idx = batch * c * h * w
                                        + channel * h * w
                                        + ih * w
                                        + iw;
                                    let val = *src_ptr.add(src_idx);

                                    pool_val = match op {
                                        PoolOp::Max => pool_val.max(val),
                                        PoolOp::Avg | PoolOp::Sum => pool_val + val,
                                    };
                                    count += 1;
                                }
                            }

                            if op == PoolOp::Avg && count > 0 {
                                pool_val /= count as f64;
                            }

                            let dst_idx = batch * c * h_out * w_out
                                + channel * h_out * w_out
                                + oh * w_out
                                + ow;
                            *dst_ptr.add(dst_idx) = pool_val;
                        }
                    }
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// 2D convolution.
///
/// Input: [N, C_in, H, W]
/// Kernel: [C_out, C_in/groups, kH, kW]
/// Bias: [C_out] (optional)
/// Output: [N, C_out, H_out, W_out]
pub fn tensor_conv2d(
    input: &TensorHandle,
    kernel: &TensorHandle,
    bias: Option<&TensorHandle>,
    stride: (usize, usize),
    padding: (usize, usize),
    dilation: (usize, usize),
    groups: usize,
) -> Option<TensorHandle> {
    if input.ndim != 4 || kernel.ndim != 4 {
        return None;
    }

    let (n, c_in, h, w) = (
        input.shape[0],
        input.shape[1],
        input.shape[2],
        input.shape[3],
    );
    let (c_out, kc_in, kh, kw) = (
        kernel.shape[0],
        kernel.shape[1],
        kernel.shape[2],
        kernel.shape[3],
    );

    // Validate groups
    if c_in != kc_in * groups || c_out % groups != 0 {
        return None;
    }

    let (sh, sw) = stride;
    let (ph, pw) = padding;
    let (dh, dw) = dilation;

    // Calculate output dimensions
    let h_out = (h + 2 * ph - dh * (kh - 1) - 1) / sh + 1;
    let w_out = (w + 2 * pw - dw * (kw - 1) - 1) / sw + 1;

    let result = TensorHandle::zeros(&[n, c_out, h_out, w_out], input.dtype)?;

    let input_data = input.data.as_ref()?;
    let kernel_data = kernel.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let c_out_per_group = c_out / groups;
    let c_in_per_group = c_in / groups;

    match input.dtype {
        DType::F32 => unsafe {
            let input_ptr = (*input_data.as_ptr()).as_ptr() as *const f32;
            let kernel_ptr = (*kernel_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;
            let bias_ptr = bias
                .and_then(|b| b.data.as_ref())
                .map(|d| (*d.as_ptr()).as_ptr() as *const f32);

            for batch in 0..n {
                for group in 0..groups {
                    for oc in 0..c_out_per_group {
                        let oc_global = group * c_out_per_group + oc;

                        for oh in 0..h_out {
                            for ow in 0..w_out {
                                let mut sum = 0.0f32;

                                for ic in 0..c_in_per_group {
                                    let ic_global = group * c_in_per_group + ic;

                                    for ikh in 0..kh {
                                        for ikw in 0..kw {
                                            let ih = oh * sh + ikh * dh;
                                            let iw = ow * sw + ikw * dw;

                                            // Handle padding
                                            let in_h = if ih >= ph && ih < h + ph {
                                                ih - ph
                                            } else {
                                                continue;
                                            };
                                            let in_w = if iw >= pw && iw < w + pw {
                                                iw - pw
                                            } else {
                                                continue;
                                            };

                                            if in_h < h && in_w < w {
                                                let input_idx = batch * c_in * h * w
                                                    + ic_global * h * w
                                                    + in_h * w
                                                    + in_w;
                                                let kernel_idx = oc_global * kc_in * kh * kw
                                                    + ic * kh * kw
                                                    + ikh * kw
                                                    + ikw;

                                                sum += *input_ptr.add(input_idx)
                                                    * *kernel_ptr.add(kernel_idx);
                                            }
                                        }
                                    }
                                }

                                // Add bias
                                if let Some(bp) = bias_ptr {
                                    sum += *bp.add(oc_global);
                                }

                                let dst_idx = batch * c_out * h_out * w_out
                                    + oc_global * h_out * w_out
                                    + oh * w_out
                                    + ow;
                                *dst_ptr.add(dst_idx) = sum;
                            }
                        }
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let input_ptr = (*input_data.as_ptr()).as_ptr() as *const f64;
            let kernel_ptr = (*kernel_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;
            let bias_ptr = bias
                .and_then(|b| b.data.as_ref())
                .map(|d| (*d.as_ptr()).as_ptr() as *const f64);

            for batch in 0..n {
                for group in 0..groups {
                    for oc in 0..c_out_per_group {
                        let oc_global = group * c_out_per_group + oc;

                        for oh in 0..h_out {
                            for ow in 0..w_out {
                                let mut sum = 0.0f64;

                                for ic in 0..c_in_per_group {
                                    let ic_global = group * c_in_per_group + ic;

                                    for ikh in 0..kh {
                                        for ikw in 0..kw {
                                            let ih = oh * sh + ikh * dh;
                                            let iw = ow * sw + ikw * dw;

                                            let in_h = if ih >= ph && ih < h + ph {
                                                ih - ph
                                            } else {
                                                continue;
                                            };
                                            let in_w = if iw >= pw && iw < w + pw {
                                                iw - pw
                                            } else {
                                                continue;
                                            };

                                            if in_h < h && in_w < w {
                                                let input_idx = batch * c_in * h * w
                                                    + ic_global * h * w
                                                    + in_h * w
                                                    + in_w;
                                                let kernel_idx = oc_global * kc_in * kh * kw
                                                    + ic * kh * kw
                                                    + ikh * kw
                                                    + ikw;

                                                sum += *input_ptr.add(input_idx)
                                                    * *kernel_ptr.add(kernel_idx);
                                            }
                                        }
                                    }
                                }

                                if let Some(bp) = bias_ptr {
                                    sum += *bp.add(oc_global);
                                }

                                let dst_idx = batch * c_out * h_out * w_out
                                    + oc_global * h_out * w_out
                                    + oh * w_out
                                    + ow;
                                *dst_ptr.add(dst_idx) = sum;
                            }
                        }
                    }
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Element-wise comparison operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum CompareOp {
    Eq,  // ==
    Ne,  // !=
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
}

/// Element-wise comparison.
///
/// Returns a boolean tensor with the comparison result.
pub fn tensor_cmp(
    a: &TensorHandle,
    b: &TensorHandle,
    op: CompareOp,
) -> Option<TensorHandle> {
    // Check broadcastable shapes
    let a_shape = &a.shape[..a.ndim as usize];
    let b_shape = &b.shape[..b.ndim as usize];
    let out_shape = broadcast_shapes(a_shape, b_shape)?;

    let result = TensorHandle::zeros(&out_shape, DType::Bool)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let out_numel: usize = out_shape.iter().product();

    match (a.dtype, b.dtype) {
        (DType::F32, DType::F32) => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f32;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

            for i in 0..out_numel {
                let a_idx = broadcast_index(i, &out_shape, a_shape);
                let b_idx = broadcast_index(i, &out_shape, b_shape);
                let av = *a_ptr.add(a_idx);
                let bv = *b_ptr.add(b_idx);

                let cmp_result = match op {
                    CompareOp::Eq => av == bv,
                    CompareOp::Ne => av != bv,
                    CompareOp::Lt => av < bv,
                    CompareOp::Le => av <= bv,
                    CompareOp::Gt => av > bv,
                    CompareOp::Ge => av >= bv,
                };
                *dst_ptr.add(i) = cmp_result as u8;
            }
        },
        (DType::F64, DType::F64) => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const f64;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

            for i in 0..out_numel {
                let a_idx = broadcast_index(i, &out_shape, a_shape);
                let b_idx = broadcast_index(i, &out_shape, b_shape);
                let av = *a_ptr.add(a_idx);
                let bv = *b_ptr.add(b_idx);

                let cmp_result = match op {
                    CompareOp::Eq => av == bv,
                    CompareOp::Ne => av != bv,
                    CompareOp::Lt => av < bv,
                    CompareOp::Le => av <= bv,
                    CompareOp::Gt => av > bv,
                    CompareOp::Ge => av >= bv,
                };
                *dst_ptr.add(i) = cmp_result as u8;
            }
        },
        (DType::I32, DType::I32) => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const i32;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const i32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

            for i in 0..out_numel {
                let a_idx = broadcast_index(i, &out_shape, a_shape);
                let b_idx = broadcast_index(i, &out_shape, b_shape);
                let av = *a_ptr.add(a_idx);
                let bv = *b_ptr.add(b_idx);

                let cmp_result = match op {
                    CompareOp::Eq => av == bv,
                    CompareOp::Ne => av != bv,
                    CompareOp::Lt => av < bv,
                    CompareOp::Le => av <= bv,
                    CompareOp::Gt => av > bv,
                    CompareOp::Ge => av >= bv,
                };
                *dst_ptr.add(i) = cmp_result as u8;
            }
        },
        (DType::I64, DType::I64) => unsafe {
            let a_ptr = (*a_data.as_ptr()).as_ptr() as *const i64;
            let b_ptr = (*b_data.as_ptr()).as_ptr() as *const i64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

            for i in 0..out_numel {
                let a_idx = broadcast_index(i, &out_shape, a_shape);
                let b_idx = broadcast_index(i, &out_shape, b_shape);
                let av = *a_ptr.add(a_idx);
                let bv = *b_ptr.add(b_idx);

                let cmp_result = match op {
                    CompareOp::Eq => av == bv,
                    CompareOp::Ne => av != bv,
                    CompareOp::Lt => av < bv,
                    CompareOp::Le => av <= bv,
                    CompareOp::Gt => av > bv,
                    CompareOp::Ge => av >= bv,
                };
                *dst_ptr.add(i) = cmp_result as u8;
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Helper: broadcast shapes for binary operations
fn broadcast_shapes(a: &[usize], b: &[usize]) -> Option<Vec<usize>> {
    let max_ndim = a.len().max(b.len());
    let mut result = Vec::with_capacity(max_ndim);

    for i in 0..max_ndim {
        let dim_a = if i < max_ndim - a.len() {
            1
        } else {
            a[i - (max_ndim - a.len())]
        };
        let dim_b = if i < max_ndim - b.len() {
            1
        } else {
            b[i - (max_ndim - b.len())]
        };

        if dim_a == dim_b {
            result.push(dim_a);
        } else if dim_a == 1 {
            result.push(dim_b);
        } else if dim_b == 1 {
            result.push(dim_a);
        } else {
            return None; // Incompatible shapes
        }
    }

    Some(result)
}

/// Helper: compute source index for broadcasting
fn broadcast_index(out_idx: usize, out_shape: &[usize], src_shape: &[usize]) -> usize {
    let out_ndim = out_shape.len();
    let src_ndim = src_shape.len();

    // Convert flat index to coordinates
    let mut coords = vec![0usize; out_ndim];
    let mut remaining = out_idx;
    for i in (0..out_ndim).rev() {
        coords[i] = remaining % out_shape[i];
        remaining /= out_shape[i];
    }

    // Adjust coordinates for source shape (broadcasting)
    let mut src_idx = 0;
    let mut src_stride = 1;
    for i in (0..src_ndim).rev() {
        let out_i = out_ndim - src_ndim + i;
        let coord = if src_shape[i] == 1 { 0 } else { coords[out_i] };
        src_idx += coord * src_stride;
        src_stride *= src_shape[i];
    }

    src_idx
}

/// Masked fill operation.
///
/// Fills elements where mask is true with the given value.
pub fn tensor_masked_fill(
    src: &TensorHandle,
    mask: &TensorHandle,
    value: f64,
) -> Option<TensorHandle> {
    // Mask must be boolean
    if mask.dtype != DType::Bool {
        return None;
    }

    // Check broadcastable
    let src_shape = &src.shape[..src.ndim as usize];
    let mask_shape = &mask.shape[..mask.ndim as usize];
    let out_shape = broadcast_shapes(src_shape, mask_shape)?;

    let result = TensorHandle::zeros(&out_shape, src.dtype)?;

    let src_data = src.data.as_ref()?;
    let mask_data = mask.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let out_numel: usize = out_shape.iter().product();

    match src.dtype {
        DType::F32 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let mask_ptr = (*mask_data.as_ptr()).as_ptr();
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;

            let fill_val = value as f32;
            for i in 0..out_numel {
                let src_idx = broadcast_index(i, &out_shape, src_shape);
                let mask_idx = broadcast_index(i, &out_shape, mask_shape);

                if *mask_ptr.add(mask_idx) != 0 {
                    *dst_ptr.add(i) = fill_val;
                } else {
                    *dst_ptr.add(i) = *src_ptr.add(src_idx);
                }
            }
        },
        DType::F64 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let mask_ptr = (*mask_data.as_ptr()).as_ptr();
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;

            for i in 0..out_numel {
                let src_idx = broadcast_index(i, &out_shape, src_shape);
                let mask_idx = broadcast_index(i, &out_shape, mask_shape);

                if *mask_ptr.add(mask_idx) != 0 {
                    *dst_ptr.add(i) = value;
                } else {
                    *dst_ptr.add(i) = *src_ptr.add(src_idx);
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Index selection along an axis.
///
/// Selects elements from src at the given indices along the specified axis.
pub fn tensor_index_select(
    src: &TensorHandle,
    indices: &TensorHandle,
    axis: usize,
) -> Option<TensorHandle> {
    if src.ndim == 0 || axis >= src.ndim as usize {
        return None;
    }

    // Indices must be integer type
    if indices.dtype != DType::I32 && indices.dtype != DType::I64 {
        return None;
    }

    let src_ndim = src.ndim as usize;
    let num_indices = indices.numel;

    // Build output shape: replace axis dimension with num_indices
    let mut out_shape = src.shape[..src_ndim].to_vec();
    out_shape[axis] = num_indices;

    let result = TensorHandle::zeros(&out_shape, src.dtype)?;

    let src_data = src.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;
    let dst_data = result.data.as_ref()?;

    let outer_size: usize = src.shape[..axis].iter().product();
    let axis_size = src.shape[axis];
    let inner_size: usize = src.shape[axis + 1..src_ndim].iter().product();

    match src.dtype {
        DType::F32 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;
            let idx_ptr = (*idx_data.as_ptr()).as_ptr();

            for outer in 0..outer_size {
                for (out_i, ii) in (0..num_indices).enumerate() {
                    let idx = if indices.dtype == DType::I64 {
                        *(idx_ptr as *const i64).add(ii) as usize
                    } else {
                        *(idx_ptr as *const i32).add(ii) as usize
                    };

                    if idx >= axis_size {
                        continue; // Skip invalid indices
                    }

                    for inner in 0..inner_size {
                        let src_idx = outer * axis_size * inner_size + idx * inner_size + inner;
                        let dst_idx =
                            outer * num_indices * inner_size + out_i * inner_size + inner;
                        *dst_ptr.add(dst_idx) = *src_ptr.add(src_idx);
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;
            let idx_ptr = (*idx_data.as_ptr()).as_ptr();

            for outer in 0..outer_size {
                for (out_i, ii) in (0..num_indices).enumerate() {
                    let idx = if indices.dtype == DType::I64 {
                        *(idx_ptr as *const i64).add(ii) as usize
                    } else {
                        *(idx_ptr as *const i32).add(ii) as usize
                    };

                    if idx >= axis_size {
                        continue;
                    }

                    for inner in 0..inner_size {
                        let src_idx = outer * axis_size * inner_size + idx * inner_size + inner;
                        let dst_idx =
                            outer * num_indices * inner_size + out_i * inner_size + inner;
                        *dst_ptr.add(dst_idx) = *src_ptr.add(src_idx);
                    }
                }
            }
        },
        _ => return None,
    }

    Some(result)
}

/// Scatter operation.
///
/// Scatters values into dst at indices along the specified axis.
pub fn tensor_scatter(
    dst: &mut TensorHandle,
    src: &TensorHandle,
    indices: &TensorHandle,
    axis: usize,
) -> Option<()> {
    if dst.ndim == 0 || axis >= dst.ndim as usize {
        return None;
    }

    // Validate shapes match except for axis
    let dst_ndim = dst.ndim as usize;
    if src.ndim as usize != dst_ndim || indices.ndim as usize != dst_ndim {
        return None;
    }

    let dst_data = dst.data.as_ref()?;
    let src_data = src.data.as_ref()?;
    let idx_data = indices.data.as_ref()?;

    let outer_size: usize = dst.shape[..axis].iter().product();
    let axis_size = dst.shape[axis];
    let src_axis_size = src.shape[axis];
    let inner_size: usize = dst.shape[axis + 1..dst_ndim].iter().product();

    match dst.dtype {
        DType::F32 => unsafe {
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f32;
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f32;
            let idx_ptr = (*idx_data.as_ptr()).as_ptr();

            for outer in 0..outer_size {
                for src_i in 0..src_axis_size {
                    for inner in 0..inner_size {
                        let idx_flat = outer * src_axis_size * inner_size + src_i * inner_size + inner;
                        let idx = if indices.dtype == DType::I64 {
                            *(idx_ptr as *const i64).add(idx_flat) as usize
                        } else {
                            *(idx_ptr as *const i32).add(idx_flat) as usize
                        };

                        if idx >= axis_size {
                            continue;
                        }

                        let src_idx = idx_flat;
                        let dst_idx = outer * axis_size * inner_size + idx * inner_size + inner;
                        *dst_ptr.add(dst_idx) = *src_ptr.add(src_idx);
                    }
                }
            }
        },
        DType::F64 => unsafe {
            let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr() as *mut f64;
            let src_ptr = (*src_data.as_ptr()).as_ptr() as *const f64;
            let idx_ptr = (*idx_data.as_ptr()).as_ptr();

            for outer in 0..outer_size {
                for src_i in 0..src_axis_size {
                    for inner in 0..inner_size {
                        let idx_flat = outer * src_axis_size * inner_size + src_i * inner_size + inner;
                        let idx = if indices.dtype == DType::I64 {
                            *(idx_ptr as *const i64).add(idx_flat) as usize
                        } else {
                            *(idx_ptr as *const i32).add(idx_flat) as usize
                        };

                        if idx >= axis_size {
                            continue;
                        }

                        let src_idx = idx_flat;
                        let dst_idx = outer * axis_size * inner_size + idx * inner_size + inner;
                        *dst_ptr.add(dst_idx) = *src_ptr.add(src_idx);
                    }
                }
            }
        },
        _ => return None,
    }

    Some(())
}

// ============================================================================
// Extended Tensor Operations (TensorExtended sub-opcodes)
// ============================================================================

/// Find argmin along axis.
///
/// Returns the index and value of the minimum element.
/// If axis is None, flattens the tensor and finds global minimum.
pub fn tensor_argmin(src: &TensorHandle, _axis: Option<i32>) -> Option<(usize, f64)> {
    if src.numel == 0 {
        return None;
    }

    let data = src.data.as_ref()?;
    let data_ptr = data.as_ptr();

    let mut min_idx = 0usize;
    let mut min_val = f64::INFINITY;

    match src.dtype {
        DType::F32 => {
            let ptr = data_ptr as *const f32;
            for i in 0..src.numel {
                let val = unsafe { *ptr.add(i) } as f64;
                if val < min_val {
                    min_val = val;
                    min_idx = i;
                }
            }
        }
        DType::F64 => {
            let ptr = data_ptr as *const f64;
            for i in 0..src.numel {
                let val = unsafe { *ptr.add(i) };
                if val < min_val {
                    min_val = val;
                    min_idx = i;
                }
            }
        }
        DType::I32 => {
            let ptr = data_ptr as *const i32;
            for i in 0..src.numel {
                let val = unsafe { *ptr.add(i) } as f64;
                if val < min_val {
                    min_val = val;
                    min_idx = i;
                }
            }
        }
        DType::I64 => {
            let ptr = data_ptr as *const i64;
            for i in 0..src.numel {
                let val = unsafe { *ptr.add(i) } as f64;
                if val < min_val {
                    min_val = val;
                    min_idx = i;
                }
            }
        }
        _ => return None,
    }

    Some((min_idx, min_val))
}

/// Permute tensor axes.
///
/// Reorders the axes of the tensor according to the given permutation.
/// For example, permute([0,1,2], [2,0,1]) swaps to [2,0,1] order.
pub fn tensor_permute(src: &TensorHandle, axes: &[usize]) -> Option<TensorHandle> {
    let ndim = src.ndim as usize;
    if axes.len() != ndim {
        return None;
    }

    // Validate axes
    let mut seen = vec![false; ndim];
    for &axis in axes {
        if axis >= ndim || seen[axis] {
            return None;
        }
        seen[axis] = true;
    }

    // Compute new shape
    let mut new_shape = Vec::with_capacity(ndim);
    for &axis in axes {
        new_shape.push(src.shape[axis]);
    }

    // Compute new strides
    let mut new_strides = Vec::with_capacity(ndim);
    for &axis in axes {
        new_strides.push(src.strides[axis]);
    }

    // Allocate result
    let mut result = TensorHandle::zeros(&new_shape, src.dtype)?;
    let result_data = result.data.as_mut()?;
    let src_data = src.data.as_ref()?;

    // Copy data with permuted indexing
    // Use signed arithmetic to support negative strides
    let numel = src.numel;
    let elem_size = src.dtype.size();

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*result_data.as_ptr()).as_mut_ptr();

        for flat_idx in 0..numel {
            // Convert flat index to multi-dimensional index in result
            let mut remaining = flat_idx as isize;
            let mut src_offset = 0isize;

            for (dim, &new_stride) in new_strides.iter().enumerate() {
                let _size = new_shape[dim];
                // For newly allocated tensors, strides are always positive
                let stride = result.strides[dim];
                debug_assert!(stride > 0, "Result tensor should have positive strides");
                let coord = remaining / stride;
                remaining %= stride;
                src_offset += coord * new_stride;
            }

            debug_assert!(src_offset >= 0, "Negative source offset indicates invalid permutation");
            let dst_offset = flat_idx;

            // Copy element
            std::ptr::copy_nonoverlapping(
                src_ptr.offset(src_offset * (elem_size as isize)),
                dst_ptr.add(dst_offset * elem_size),
                elem_size,
            );
        }
    }

    Some(result)
}

/// Compute matrix determinant.
///
/// Uses LU decomposition to compute the determinant.
/// Only works on 2D square matrices.
pub fn tensor_det(src: &TensorHandle) -> Option<f64> {
    if src.ndim != 2 || src.shape[0] != src.shape[1] {
        return None;
    }

    let n = src.shape[0];
    if n == 0 {
        return Some(1.0);
    }

    let data = src.data.as_ref()?;
    let data_ptr = data.as_ptr();

    // Copy to working matrix
    let mut matrix: Vec<f64> = Vec::with_capacity(n * n);

    match src.dtype {
        DType::F32 => {
            let ptr = data_ptr as *const f32;
            for i in 0..(n * n) {
                matrix.push(unsafe { *ptr.add(i) } as f64);
            }
        }
        DType::F64 => {
            let ptr = data_ptr as *const f64;
            for i in 0..(n * n) {
                matrix.push(unsafe { *ptr.add(i) });
            }
        }
        _ => return None,
    }

    // LU decomposition in-place
    let mut det = 1.0;
    let mut sign = 1.0;

    for k in 0..n {
        // Find pivot
        let mut max_val = matrix[k * n + k].abs();
        let mut max_row = k;

        for i in (k + 1)..n {
            let val = matrix[i * n + k].abs();
            if val > max_val {
                max_val = val;
                max_row = i;
            }
        }

        if max_val < 1e-15 {
            return Some(0.0); // Singular matrix
        }

        // Swap rows if needed
        if max_row != k {
            for j in 0..n {
                matrix.swap(k * n + j, max_row * n + j);
            }
            sign = -sign;
        }

        det *= matrix[k * n + k];

        // Eliminate column
        for i in (k + 1)..n {
            let factor = matrix[i * n + k] / matrix[k * n + k];
            for j in k..n {
                matrix[i * n + j] -= factor * matrix[k * n + j];
            }
        }
    }

    Some(det * sign)
}

/// Compute matrix trace.
///
/// Returns the sum of diagonal elements.
pub fn tensor_trace(src: &TensorHandle) -> Option<f64> {
    if src.ndim != 2 {
        return None;
    }

    let n = src.shape[0].min(src.shape[1]);
    if n == 0 {
        return Some(0.0);
    }

    let data = src.data.as_ref()?;
    let data_ptr = data.as_ptr();
    let cols = src.shape[1];

    let mut trace = 0.0;

    match src.dtype {
        DType::F32 => {
            let ptr = data_ptr as *const f32;
            for i in 0..n {
                trace += unsafe { *ptr.add(i * cols + i) } as f64;
            }
        }
        DType::F64 => {
            let ptr = data_ptr as *const f64;
            for i in 0..n {
                trace += unsafe { *ptr.add(i * cols + i) };
            }
        }
        DType::I32 => {
            let ptr = data_ptr as *const i32;
            for i in 0..n {
                trace += unsafe { *ptr.add(i * cols + i) } as f64;
            }
        }
        DType::I64 => {
            let ptr = data_ptr as *const i64;
            for i in 0..n {
                trace += unsafe { *ptr.add(i * cols + i) } as f64;
            }
        }
        _ => return None,
    }

    Some(trace)
}

/// Compute tensor/matrix norm.
///
/// For vectors:
///   - ord = 0: L0 norm (count non-zero)
///   - ord = 1: L1 norm (sum of absolute values)
///   - ord = 2: L2 norm (Euclidean norm)
///   - ord = -1: L-inf norm (max absolute value)
///
/// For matrices:
///   - ord = 0: Frobenius norm
///   - ord = 1: max column sum
///   - ord = 2: spectral norm (largest singular value) - approximated
///   - ord = -1: max row sum
pub fn tensor_norm(src: &TensorHandle, ord: i8) -> Option<f64> {
    if src.numel == 0 {
        return Some(0.0);
    }

    let data = src.data.as_ref()?;
    let data_ptr = data.as_ptr();

    // Get values as f64
    let values: Vec<f64> = match src.dtype {
        DType::F32 => {
            let ptr = data_ptr as *const f32;
            (0..src.numel)
                .map(|i| unsafe { *ptr.add(i) } as f64)
                .collect()
        }
        DType::F64 => {
            let ptr = data_ptr as *const f64;
            (0..src.numel).map(|i| unsafe { *ptr.add(i) }).collect()
        }
        DType::I32 => {
            let ptr = data_ptr as *const i32;
            (0..src.numel)
                .map(|i| unsafe { *ptr.add(i) } as f64)
                .collect()
        }
        DType::I64 => {
            let ptr = data_ptr as *const i64;
            (0..src.numel)
                .map(|i| unsafe { *ptr.add(i) } as f64)
                .collect()
        }
        _ => return None,
    };

    match ord {
        0 => {
            // Frobenius / L2 squared for vectors
            let sum_sq: f64 = values.iter().map(|x| x * x).sum();
            Some(sum_sq.sqrt())
        }
        1 => {
            // L1 norm
            let sum: f64 = values.iter().map(|x| x.abs()).sum();
            Some(sum)
        }
        2 => {
            // L2 norm
            let sum_sq: f64 = values.iter().map(|x| x * x).sum();
            Some(sum_sq.sqrt())
        }
        -1 => {
            // L-inf norm (max absolute value)
            let max = values.iter().map(|x| x.abs()).fold(0.0f64, f64::max);
            Some(max)
        }
        -2 => {
            // Min absolute value (non-zero)
            let min = values
                .iter()
                .map(|x| x.abs())
                .filter(|&x| x > 0.0)
                .fold(f64::INFINITY, f64::min);
            if min == f64::INFINITY {
                Some(0.0)
            } else {
                Some(min)
            }
        }
        _ => {
            // Default to L2
            let sum_sq: f64 = values.iter().map(|x| x * x).sum();
            Some(sum_sq.sqrt())
        }
    }
}

// ============================================================================
// Additional Shape Operations
// ============================================================================

/// Unsqueeze: insert a dimension of size 1 at the specified position.
///
/// Inverse of squeeze. dim can be negative for counting from the end.
/// Example: tensor of shape [3, 4] with dim=0 → [1, 3, 4]
///          tensor of shape [3, 4] with dim=1 → [3, 1, 4]
///          tensor of shape [3, 4] with dim=-1 → [3, 4, 1]
pub fn tensor_unsqueeze(src: &TensorHandle, dim: i32) -> Option<TensorHandle> {
    if src.ndim >= 8 {
        return None; // Max dimensions reached
    }

    let ndim = src.ndim as i32;
    let actual_dim = if dim < 0 {
        (ndim + 1 + dim) as usize
    } else {
        dim as usize
    };

    if actual_dim > ndim as usize {
        return None;
    }

    let mut new_shape = Vec::with_capacity(ndim as usize + 1);
    for i in 0..actual_dim {
        new_shape.push(src.shape[i]);
    }
    new_shape.push(1);
    for i in actual_dim..(ndim as usize) {
        new_shape.push(src.shape[i]);
    }

    tensor_reshape(src, &new_shape)
}

/// Split a tensor into multiple tensors along an axis.
///
/// Returns a vector of tensors. Each tensor has the same dtype as input.
/// If the tensor cannot be evenly split, the last chunk will be smaller.
pub fn tensor_split(src: &TensorHandle, num_or_sizes: &[usize], axis: usize) -> Option<Vec<TensorHandle>> {
    if axis >= src.ndim as usize {
        return None;
    }

    let axis_size = src.shape[axis];
    let shape = &src.shape[..src.ndim as usize];

    // Calculate chunk sizes
    let chunk_sizes: Vec<usize> = if num_or_sizes.len() == 1 {
        // Split into equal chunks
        let num_chunks = num_or_sizes[0];
        if num_chunks == 0 {
            return None;
        }
        let chunk_size = axis_size / num_chunks;
        let remainder = axis_size % num_chunks;
        let mut sizes = vec![chunk_size; num_chunks];
        for size in sizes.iter_mut().take(remainder) {
            *size += 1;
        }
        sizes
    } else {
        // Use provided sizes
        num_or_sizes.to_vec()
    };

    let total: usize = chunk_sizes.iter().sum();
    if total != axis_size {
        return None;
    }

    let mut results = Vec::with_capacity(chunk_sizes.len());
    let mut offset = 0usize;

    for chunk_size in &chunk_sizes {
        if *chunk_size == 0 {
            continue;
        }

        // Build slice ranges
        let mut ranges: Vec<(usize, usize)> = shape.iter().map(|&s| (0, s)).collect();
        ranges[axis] = (offset, offset + chunk_size);

        if let Some(chunk) = tensor_slice(src, &ranges) {
            results.push(chunk);
        } else {
            return None;
        }

        offset += chunk_size;
    }

    Some(results)
}

/// Split a tensor at a specific position along an axis.
///
/// Returns a tuple of two tensors: (left, right)
/// where left has indices [0, pos) and right has [pos, size)
pub fn tensor_split_at(src: &TensorHandle, pos: usize, axis: usize) -> Option<(TensorHandle, TensorHandle)> {
    if axis >= src.ndim as usize {
        return None;
    }

    let axis_size = src.shape[axis];
    if pos > axis_size {
        return None;
    }

    let shape = &src.shape[..src.ndim as usize];

    // Left part
    let mut left_ranges: Vec<(usize, usize)> = shape.iter().map(|&s| (0, s)).collect();
    left_ranges[axis] = (0, pos);

    // Right part
    let mut right_ranges: Vec<(usize, usize)> = shape.iter().map(|&s| (0, s)).collect();
    right_ranges[axis] = (pos, axis_size);

    let left = if pos > 0 {
        tensor_slice(src, &left_ranges)?
    } else {
        TensorHandle::zeros(&[0], src.dtype)?
    };

    let right = if pos < axis_size {
        tensor_slice(src, &right_ranges)?
    } else {
        TensorHandle::zeros(&[0], src.dtype)?
    };

    Some((left, right))
}

/// Gather elements along an axis using indices.
///
/// output[i][j][k] = input[index[i][j][k]][j][k] (for axis=0)
pub fn tensor_gather(src: &TensorHandle, indices: &TensorHandle, axis: i8) -> Option<TensorHandle> {
    use super::kernel::cpu::{gather_f32_scalar, gather_f64_scalar};

    match src.dtype {
        DType::F32 => gather_f32_scalar(src, indices, axis),
        DType::F64 => gather_f64_scalar(src, indices, axis),
        _ => None, // Other dtypes not yet supported
    }
}

/// Repeat tensor along each dimension.
///
/// repeats[i] specifies how many times to repeat along dimension i.
/// Example: tensor [2, 3] with repeats [2, 3] → [4, 9]
pub fn tensor_repeat(src: &TensorHandle, repeats: &[usize]) -> Option<TensorHandle> {
    if repeats.len() != src.ndim as usize {
        return None;
    }

    let shape = &src.shape[..src.ndim as usize];
    let new_shape: Vec<usize> = shape.iter().zip(repeats).map(|(&s, &r)| s * r).collect();

    let mut output = TensorHandle::zeros(&new_shape, src.dtype)?;
    let n = output.numel;

    // Compute strides
    let mut src_strides = vec![1usize; src.ndim as usize];
    for i in (0..(src.ndim as usize - 1)).rev() {
        src_strides[i] = src_strides[i + 1] * shape[i + 1];
    }

    let mut out_strides = vec![1usize; new_shape.len()];
    for i in (0..(new_shape.len() - 1)).rev() {
        out_strides[i] = out_strides[i + 1] * new_shape[i + 1];
    }

    match src.dtype {
        DType::F32 => {
            let src_ptr = src.data_ptr_f32();
            let out_ptr = output.data_ptr_f32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    // Convert output index to coordinates, then wrap to source
                    let mut src_idx = 0usize;
                    let mut remaining = out_idx;
                    for d in 0..new_shape.len() {
                        let coord = remaining / out_strides[d];
                        remaining %= out_strides[d];
                        let src_coord = coord % shape[d];
                        src_idx += src_coord * src_strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_idx);
                }
            }
        }
        DType::F64 => {
            let src_ptr = src.data_ptr_f64();
            let out_ptr = output.data_ptr_f64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut src_idx = 0usize;
                    let mut remaining = out_idx;
                    for d in 0..new_shape.len() {
                        let coord = remaining / out_strides[d];
                        remaining %= out_strides[d];
                        let src_coord = coord % shape[d];
                        src_idx += src_coord * src_strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_idx);
                }
            }
        }
        DType::I32 => {
            let src_ptr = src.data_ptr_i32();
            let out_ptr = output.data_ptr_i32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut src_idx = 0usize;
                    let mut remaining = out_idx;
                    for d in 0..new_shape.len() {
                        let coord = remaining / out_strides[d];
                        remaining %= out_strides[d];
                        let src_coord = coord % shape[d];
                        src_idx += src_coord * src_strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_idx);
                }
            }
        }
        DType::I64 => {
            let src_ptr = src.data_ptr_i64();
            let out_ptr = output.data_ptr_i64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut src_idx = 0usize;
                    let mut remaining = out_idx;
                    for d in 0..new_shape.len() {
                        let coord = remaining / out_strides[d];
                        remaining %= out_strides[d];
                        let src_coord = coord % shape[d];
                        src_idx += src_coord * src_strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_idx);
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

/// Select elements from tensor where mask is true.
///
/// Returns a 1D tensor containing all elements where mask is true.
/// mask must be a Bool tensor with the same shape as src.
pub fn tensor_masked_select(src: &TensorHandle, mask: &TensorHandle) -> Option<TensorHandle> {
    if mask.dtype != DType::Bool {
        return None;
    }

    if src.numel != mask.numel {
        return None;
    }

    let mask_data = mask.data.as_ref()?;
    let mask_ptr = mask_data.as_ptr() as *const u8;

    // Count true values
    let mut count = 0usize;
    unsafe {
        for i in 0..mask.numel {
            if *mask_ptr.add(i) != 0 {
                count += 1;
            }
        }
    }

    let mut output = TensorHandle::zeros(&[count], src.dtype)?;

    if count == 0 {
        return Some(output);
    }

    match src.dtype {
        DType::F32 => {
            let src_ptr = src.data_ptr_f32();
            let out_ptr = output.data_ptr_f32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                let mut out_idx = 0;
                for i in 0..src.numel {
                    if *mask_ptr.add(i) != 0 {
                        *out_ptr.add(out_idx) = *src_ptr.add(i);
                        out_idx += 1;
                    }
                }
            }
        }
        DType::F64 => {
            let src_ptr = src.data_ptr_f64();
            let out_ptr = output.data_ptr_f64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                let mut out_idx = 0;
                for i in 0..src.numel {
                    if *mask_ptr.add(i) != 0 {
                        *out_ptr.add(out_idx) = *src_ptr.add(i);
                        out_idx += 1;
                    }
                }
            }
        }
        DType::I32 => {
            let src_ptr = src.data_ptr_i32();
            let out_ptr = output.data_ptr_i32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                let mut out_idx = 0;
                for i in 0..src.numel {
                    if *mask_ptr.add(i) != 0 {
                        *out_ptr.add(out_idx) = *src_ptr.add(i);
                        out_idx += 1;
                    }
                }
            }
        }
        DType::I64 => {
            let src_ptr = src.data_ptr_i64();
            let out_ptr = output.data_ptr_i64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                let mut out_idx = 0;
                for i in 0..src.numel {
                    if *mask_ptr.add(i) != 0 {
                        *out_ptr.add(out_idx) = *src_ptr.add(i);
                        out_idx += 1;
                    }
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

/// Find indices of non-zero elements.
///
/// Returns a 2D tensor of shape [num_nonzero, ndim] containing the indices.
/// For 1D tensors, returns shape [num_nonzero, 1].
pub fn tensor_nonzero(src: &TensorHandle) -> Option<TensorHandle> {
    let shape = &src.shape[..src.ndim as usize];
    let ndim = src.ndim as usize;

    // Compute strides for index computation
    let mut strides = vec![1usize; ndim];
    for i in (0..(ndim - 1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }

    // Find non-zero indices based on dtype
    let mut indices: Vec<Vec<i64>> = Vec::new();

    match src.dtype {
        DType::F32 => {
            let ptr = src.data_ptr_f32();
            if ptr.is_null() {
                return None;
            }
            unsafe {
                for flat_idx in 0..src.numel {
                    if *ptr.add(flat_idx) != 0.0 {
                        let mut multi_idx = vec![0i64; ndim];
                        let mut remaining = flat_idx;
                        for d in 0..ndim {
                            multi_idx[d] = (remaining / strides[d]) as i64;
                            remaining %= strides[d];
                        }
                        indices.push(multi_idx);
                    }
                }
            }
        }
        DType::F64 => {
            let ptr = src.data_ptr_f64();
            if ptr.is_null() {
                return None;
            }
            unsafe {
                for flat_idx in 0..src.numel {
                    if *ptr.add(flat_idx) != 0.0 {
                        let mut multi_idx = vec![0i64; ndim];
                        let mut remaining = flat_idx;
                        for d in 0..ndim {
                            multi_idx[d] = (remaining / strides[d]) as i64;
                            remaining %= strides[d];
                        }
                        indices.push(multi_idx);
                    }
                }
            }
        }
        DType::I32 => {
            let ptr = src.data_ptr_i32();
            if ptr.is_null() {
                return None;
            }
            unsafe {
                for flat_idx in 0..src.numel {
                    if *ptr.add(flat_idx) != 0 {
                        let mut multi_idx = vec![0i64; ndim];
                        let mut remaining = flat_idx;
                        for d in 0..ndim {
                            multi_idx[d] = (remaining / strides[d]) as i64;
                            remaining %= strides[d];
                        }
                        indices.push(multi_idx);
                    }
                }
            }
        }
        DType::I64 => {
            let ptr = src.data_ptr_i64();
            if ptr.is_null() {
                return None;
            }
            unsafe {
                for flat_idx in 0..src.numel {
                    if *ptr.add(flat_idx) != 0 {
                        let mut multi_idx = vec![0i64; ndim];
                        let mut remaining = flat_idx;
                        for d in 0..ndim {
                            multi_idx[d] = (remaining / strides[d]) as i64;
                            remaining %= strides[d];
                        }
                        indices.push(multi_idx);
                    }
                }
            }
        }
        DType::Bool => {
            let data = src.data.as_ref()?;
            let ptr = data.as_ptr() as *const u8;
            unsafe {
                for flat_idx in 0..src.numel {
                    if *ptr.add(flat_idx) != 0 {
                        let mut multi_idx = vec![0i64; ndim];
                        let mut remaining = flat_idx;
                        for d in 0..ndim {
                            multi_idx[d] = (remaining / strides[d]) as i64;
                            remaining %= strides[d];
                        }
                        indices.push(multi_idx);
                    }
                }
            }
        }
        _ => return None,
    }

    let num_nonzero = indices.len();
    if num_nonzero == 0 {
        return TensorHandle::zeros(&[0, ndim], DType::I64);
    }

    let mut output = TensorHandle::zeros(&[num_nonzero, ndim], DType::I64)?;
    let out_ptr = output.data_ptr_i64_mut();
    if out_ptr.is_null() {
        return None;
    }

    unsafe {
        for (i, idx) in indices.iter().enumerate() {
            for (j, &val) in idx.iter().enumerate() {
                *out_ptr.add(i * ndim + j) = val;
            }
        }
    }

    Some(output)
}

/// Create one-hot encoding tensor.
///
/// indices: 1D tensor of integer indices [batch_size]
/// num_classes: number of classes (output dimension)
/// Returns: 2D tensor of shape [batch_size, num_classes]
pub fn tensor_one_hot(indices: &TensorHandle, num_classes: usize) -> Option<TensorHandle> {
    if indices.ndim != 1 {
        return None;
    }

    if !indices.dtype.is_integer() {
        return None;
    }

    let batch_size = indices.shape[0];
    let mut output = TensorHandle::zeros(&[batch_size, num_classes], DType::F32)?;
    let out_ptr = output.data_ptr_f32_mut();

    if out_ptr.is_null() {
        return None;
    }

    match indices.dtype {
        DType::I32 => {
            let idx_ptr = indices.data_ptr_i32();
            if idx_ptr.is_null() {
                return None;
            }
            unsafe {
                for i in 0..batch_size {
                    let idx = *idx_ptr.add(i) as usize;
                    if idx < num_classes {
                        *out_ptr.add(i * num_classes + idx) = 1.0;
                    }
                }
            }
        }
        DType::I64 => {
            let idx_ptr = indices.data_ptr_i64();
            if idx_ptr.is_null() {
                return None;
            }
            unsafe {
                for i in 0..batch_size {
                    let idx = *idx_ptr.add(i) as usize;
                    if idx < num_classes {
                        *out_ptr.add(i * num_classes + idx) = 1.0;
                    }
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

/// Leaky ReLU activation function.
///
/// For each element: if x >= 0, output x; otherwise output alpha * x.
pub fn tensor_leaky_relu(src: &TensorHandle, alpha: f64) -> Option<TensorHandle> {
    let shape = &src.shape[..src.ndim as usize];
    let mut result = TensorHandle::zeros(shape, src.dtype)?;

    match src.dtype {
        DType::F32 => {
            let src_ptr = src.data_ptr_f32();
            let dst_ptr = result.data_ptr_f32_mut();
            if src_ptr.is_null() || dst_ptr.is_null() {
                return None;
            }
            let alpha_f32 = alpha as f32;
            unsafe {
                for i in 0..src.numel {
                    let x = *src_ptr.add(i);
                    *dst_ptr.add(i) = if x >= 0.0 { x } else { alpha_f32 * x };
                }
            }
        }
        DType::F64 => {
            let src_ptr = src.data_ptr_f64();
            let dst_ptr = result.data_ptr_f64_mut();
            if src_ptr.is_null() || dst_ptr.is_null() {
                return None;
            }
            unsafe {
                for i in 0..src.numel {
                    let x = *src_ptr.add(i);
                    *dst_ptr.add(i) = if x >= 0.0 { x } else { alpha * x };
                }
            }
        }
        _ => return None,
    }

    Some(result)
}

/// Flip tensor along specified axes.
///
/// Reverses the order of elements along each specified axis.
pub fn tensor_flip(src: &TensorHandle, axes: &[usize]) -> Option<TensorHandle> {
    let shape = &src.shape[..src.ndim as usize];
    let ndim = src.ndim as usize;

    // Validate axes
    for &axis in axes {
        if axis >= ndim {
            return None;
        }
    }

    let mut output = TensorHandle::zeros(shape, src.dtype)?;
    let n = src.numel;

    // Compute strides
    let mut strides = vec![1usize; ndim];
    for i in (0..(ndim - 1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }

    match src.dtype {
        DType::F32 => {
            let src_ptr = src.data_ptr_f32();
            let out_ptr = output.data_ptr_f32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    // Convert flat index to multi-dimensional coordinates
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        // Flip coordinate if axis is in the flip list
                        let src_coord = if axes.contains(&d) {
                            shape[d] - 1 - coord
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::F64 => {
            let src_ptr = src.data_ptr_f64();
            let out_ptr = output.data_ptr_f64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if axes.contains(&d) {
                            shape[d] - 1 - coord
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::I32 => {
            let src_ptr = src.data_ptr_i32();
            let out_ptr = output.data_ptr_i32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if axes.contains(&d) {
                            shape[d] - 1 - coord
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::I64 => {
            let src_ptr = src.data_ptr_i64();
            let out_ptr = output.data_ptr_i64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if axes.contains(&d) {
                            shape[d] - 1 - coord
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

/// Roll tensor along axis by a shift amount.
///
/// Elements that roll beyond the last position are re-introduced at the first.
#[allow(clippy::needless_range_loop)]
pub fn tensor_roll(src: &TensorHandle, shift: i32, axis: usize) -> Option<TensorHandle> {
    let shape = &src.shape[..src.ndim as usize];
    let ndim = src.ndim as usize;

    if axis >= ndim {
        return None;
    }

    let axis_size = shape[axis] as i32;
    if axis_size == 0 {
        return tensor_clone(src);
    }

    // Normalize shift to [0, axis_size)
    let shift = ((shift % axis_size) + axis_size) % axis_size;

    let mut output = TensorHandle::zeros(shape, src.dtype)?;
    let n = src.numel;

    // Compute strides
    let mut strides = vec![1usize; ndim];
    for i in (0..(ndim - 1)).rev() {
        strides[i] = strides[i + 1] * shape[i + 1];
    }

    match src.dtype {
        DType::F32 => {
            let src_ptr = src.data_ptr_f32();
            let out_ptr = output.data_ptr_f32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if d == axis {
                            ((coord as i32 - shift + axis_size) % axis_size) as usize
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::F64 => {
            let src_ptr = src.data_ptr_f64();
            let out_ptr = output.data_ptr_f64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if d == axis {
                            ((coord as i32 - shift + axis_size) % axis_size) as usize
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::I32 => {
            let src_ptr = src.data_ptr_i32();
            let out_ptr = output.data_ptr_i32_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if d == axis {
                            ((coord as i32 - shift + axis_size) % axis_size) as usize
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        DType::I64 => {
            let src_ptr = src.data_ptr_i64();
            let out_ptr = output.data_ptr_i64_mut();
            if src_ptr.is_null() || out_ptr.is_null() {
                return None;
            }

            unsafe {
                for out_idx in 0..n {
                    let mut remaining = out_idx;
                    let mut src_flat_idx = 0usize;
                    for d in 0..ndim {
                        let coord = remaining / strides[d];
                        remaining %= strides[d];
                        let src_coord = if d == axis {
                            ((coord as i32 - shift + axis_size) % axis_size) as usize
                        } else {
                            coord
                        };
                        src_flat_idx += src_coord * strides[d];
                    }
                    *out_ptr.add(out_idx) = *src_ptr.add(src_flat_idx);
                }
            }
        }
        _ => return None,
    }

    Some(output)
}

// ============================================================================
// Boolean Logical Operations
// ============================================================================

/// Logical NOT operation on a boolean tensor.
///
/// Returns a tensor with each element negated.
pub fn tensor_logical_not(src: &TensorHandle) -> Option<TensorHandle> {
    if src.dtype != DType::Bool {
        return None;
    }

    let shape = &src.shape[..src.ndim as usize];
    let output = TensorHandle::zeros(shape, DType::Bool)?;

    let src_data = src.data.as_ref()?;
    let dst_data = output.data.as_ref()?;

    let n: usize = shape.iter().product();

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        for i in 0..n {
            *dst_ptr.add(i) = if *src_ptr.add(i) != 0 { 0 } else { 1 };
        }
    }

    Some(output)
}

/// Logical AND operation on two boolean tensors.
///
/// Supports broadcasting. Both inputs must be Bool dtype.
pub fn tensor_logical_and(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.dtype != DType::Bool || b.dtype != DType::Bool {
        return None;
    }

    let a_shape = &a.shape[..a.ndim as usize];
    let b_shape = &b.shape[..b.ndim as usize];
    let out_shape = broadcast_shapes(a_shape, b_shape)?;

    let output = TensorHandle::zeros(&out_shape, DType::Bool)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = output.data.as_ref()?;

    let n: usize = out_shape.iter().product();

    unsafe {
        let a_ptr = (*a_data.as_ptr()).as_ptr();
        let b_ptr = (*b_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        for i in 0..n {
            let a_idx = broadcast_index(i, &out_shape, a_shape);
            let b_idx = broadcast_index(i, &out_shape, b_shape);
            let av = *a_ptr.add(a_idx) != 0;
            let bv = *b_ptr.add(b_idx) != 0;
            *dst_ptr.add(i) = (av && bv) as u8;
        }
    }

    Some(output)
}

/// Logical OR operation on two boolean tensors.
///
/// Supports broadcasting. Both inputs must be Bool dtype.
pub fn tensor_logical_or(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.dtype != DType::Bool || b.dtype != DType::Bool {
        return None;
    }

    let a_shape = &a.shape[..a.ndim as usize];
    let b_shape = &b.shape[..b.ndim as usize];
    let out_shape = broadcast_shapes(a_shape, b_shape)?;

    let output = TensorHandle::zeros(&out_shape, DType::Bool)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = output.data.as_ref()?;

    let n: usize = out_shape.iter().product();

    unsafe {
        let a_ptr = (*a_data.as_ptr()).as_ptr();
        let b_ptr = (*b_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        for i in 0..n {
            let a_idx = broadcast_index(i, &out_shape, a_shape);
            let b_idx = broadcast_index(i, &out_shape, b_shape);
            let av = *a_ptr.add(a_idx) != 0;
            let bv = *b_ptr.add(b_idx) != 0;
            *dst_ptr.add(i) = (av || bv) as u8;
        }
    }

    Some(output)
}

/// Logical XOR operation on two boolean tensors.
///
/// Supports broadcasting. Both inputs must be Bool dtype.
pub fn tensor_logical_xor(a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
    if a.dtype != DType::Bool || b.dtype != DType::Bool {
        return None;
    }

    let a_shape = &a.shape[..a.ndim as usize];
    let b_shape = &b.shape[..b.ndim as usize];
    let out_shape = broadcast_shapes(a_shape, b_shape)?;

    let output = TensorHandle::zeros(&out_shape, DType::Bool)?;

    let a_data = a.data.as_ref()?;
    let b_data = b.data.as_ref()?;
    let dst_data = output.data.as_ref()?;

    let n: usize = out_shape.iter().product();

    unsafe {
        let a_ptr = (*a_data.as_ptr()).as_ptr();
        let b_ptr = (*b_data.as_ptr()).as_ptr();
        let dst_ptr = (*dst_data.as_ptr()).as_mut_ptr();

        for i in 0..n {
            let a_idx = broadcast_index(i, &out_shape, a_shape);
            let b_idx = broadcast_index(i, &out_shape, b_shape);
            let av = *a_ptr.add(a_idx) != 0;
            let bv = *b_ptr.add(b_idx) != 0;
            *dst_ptr.add(i) = (av ^ bv) as u8;
        }
    }

    Some(output)
}

/// Check if all elements in a boolean tensor are true.
///
/// Returns true if all elements are non-zero.
pub fn tensor_all(src: &TensorHandle) -> Option<bool> {
    if src.dtype != DType::Bool {
        return None;
    }

    let src_data = src.data.as_ref()?;
    let n: usize = src.shape[..src.ndim as usize].iter().product();

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();

        for i in 0..n {
            if *src_ptr.add(i) == 0 {
                return Some(false);
            }
        }
    }

    Some(true)
}

/// Check if any element in a boolean tensor is true.
///
/// Returns true if at least one element is non-zero.
pub fn tensor_any(src: &TensorHandle) -> Option<bool> {
    if src.dtype != DType::Bool {
        return None;
    }

    let src_data = src.data.as_ref()?;
    let n: usize = src.shape[..src.ndim as usize].iter().product();

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();

        for i in 0..n {
            if *src_ptr.add(i) != 0 {
                return Some(true);
            }
        }
    }

    Some(false)
}

/// Create a boolean tensor from a slice of bool values.
pub fn tensor_from_bools(data: &[bool], shape: &[usize]) -> Option<TensorHandle> {
    let expected_len: usize = shape.iter().product();
    if data.len() != expected_len {
        return None;
    }

    let tensor = TensorHandle::zeros(shape, DType::Bool)?;
    let tensor_data = tensor.data.as_ref()?;

    unsafe {
        let dst_ptr = (*tensor_data.as_ptr()).as_mut_ptr();
        for (i, &v) in data.iter().enumerate() {
            *dst_ptr.add(i) = v as u8;
        }
    }

    Some(tensor)
}

/// Convert a comparison result to boolean values.
///
/// Returns a Vec<bool> containing the comparison results.
pub fn tensor_to_bools(src: &TensorHandle) -> Option<Vec<bool>> {
    if src.dtype != DType::Bool {
        return None;
    }

    let src_data = src.data.as_ref()?;
    let n: usize = src.shape[..src.ndim as usize].iter().product();

    let mut result = Vec::with_capacity(n);

    unsafe {
        let src_ptr = (*src_data.as_ptr()).as_ptr();
        for i in 0..n {
            result.push(*src_ptr.add(i) != 0);
        }
    }

    Some(result)
}

// ============================================================================
// Flash Attention (CPU Fallback)
// ============================================================================

/// Flash Attention CPU implementation.
///
/// Computes scaled dot-product attention:
///   output = softmax(Q @ K^T / scale) @ V
///
/// With optional causal masking to prevent attending to future positions.
///
/// # Arguments
/// * `query` - Query tensor [batch, num_heads, seq_len, head_dim]
/// * `key` - Key tensor [batch, num_heads, seq_len, head_dim]
/// * `value` - Value tensor [batch, num_heads, seq_len, head_dim]
/// * `scale` - Scaling factor (typically 1/sqrt(head_dim))
/// * `causal` - Whether to apply causal mask
///
/// # Returns
/// Output tensor [batch, num_heads, seq_len, head_dim]
pub fn flash_attention_cpu(
    query: &TensorHandle,
    key: &TensorHandle,
    value: &TensorHandle,
    scale: f32,
    causal: bool,
) -> Option<TensorHandle> {
    // Validate shapes - expect [batch, num_heads, seq_len, head_dim]
    if query.ndim != 4 || key.ndim != 4 || value.ndim != 4 {
        return None;
    }

    let batch = query.shape[0];
    let num_heads = query.shape[1];
    let seq_len_q = query.shape[2];
    let head_dim = query.shape[3];
    let seq_len_kv = key.shape[2];

    // Validate K and V have consistent shapes
    if key.shape[0] != batch
        || key.shape[1] != num_heads
        || key.shape[3] != head_dim
        || value.shape[0] != batch
        || value.shape[1] != num_heads
        || value.shape[2] != seq_len_kv
        || value.shape[3] != head_dim
    {
        return None;
    }

    // Only support F32 for now
    if query.dtype != DType::F32 || key.dtype != DType::F32 || value.dtype != DType::F32 {
        return None;
    }

    let q_data = query.data.as_ref()?;
    let k_data = key.data.as_ref()?;
    let v_data = value.data.as_ref()?;

    let total_output = batch * num_heads * seq_len_q * head_dim;
    let mut output_data = vec![0.0f32; total_output];

    unsafe {
        let q_ptr = (*q_data.as_ptr()).as_ptr() as *const f32;
        let k_ptr = (*k_data.as_ptr()).as_ptr() as *const f32;
        let v_ptr = (*v_data.as_ptr()).as_ptr() as *const f32;

        // For each batch and head
        for b in 0..batch {
            for h in 0..num_heads {
                // Compute Q @ K^T for this batch/head
                // Q[b,h]: [seq_len_q, head_dim]
                // K[b,h]: [seq_len_kv, head_dim]
                // scores: [seq_len_q, seq_len_kv]

                let q_offset = (b * num_heads + h) * seq_len_q * head_dim;
                let k_offset = (b * num_heads + h) * seq_len_kv * head_dim;
                let v_offset = (b * num_heads + h) * seq_len_kv * head_dim;
                let o_offset = (b * num_heads + h) * seq_len_q * head_dim;

                // Allocate scores buffer
                let mut scores = vec![0.0f32; seq_len_q * seq_len_kv];

                // Compute scaled dot-product: Q @ K^T * scale
                for i in 0..seq_len_q {
                    for j in 0..seq_len_kv {
                        let mut dot = 0.0f32;
                        for d in 0..head_dim {
                            let q_val = *q_ptr.add(q_offset + i * head_dim + d);
                            let k_val = *k_ptr.add(k_offset + j * head_dim + d);
                            dot += q_val * k_val;
                        }
                        scores[i * seq_len_kv + j] = dot * scale;
                    }
                }

                // Apply causal mask (set future positions to -inf)
                if causal {
                    for i in 0..seq_len_q {
                        for j in (i + 1)..seq_len_kv {
                            scores[i * seq_len_kv + j] = f32::NEG_INFINITY;
                        }
                    }
                }

                // Softmax along last dimension (key dimension)
                for i in 0..seq_len_q {
                    let row_start = i * seq_len_kv;

                    // Find max for numerical stability
                    let mut max_val = f32::NEG_INFINITY;
                    for j in 0..seq_len_kv {
                        max_val = max_val.max(scores[row_start + j]);
                    }

                    // Exp and sum
                    let mut sum = 0.0f32;
                    for j in 0..seq_len_kv {
                        scores[row_start + j] = (scores[row_start + j] - max_val).exp();
                        sum += scores[row_start + j];
                    }

                    // Normalize
                    for j in 0..seq_len_kv {
                        scores[row_start + j] /= sum;
                    }
                }

                // Compute attention @ V
                // scores: [seq_len_q, seq_len_kv]
                // V[b,h]: [seq_len_kv, head_dim]
                // output: [seq_len_q, head_dim]
                for i in 0..seq_len_q {
                    for d in 0..head_dim {
                        let mut sum = 0.0f32;
                        for j in 0..seq_len_kv {
                            let attn_weight = scores[i * seq_len_kv + j];
                            let v_val = *v_ptr.add(v_offset + j * head_dim + d);
                            sum += attn_weight * v_val;
                        }
                        output_data[o_offset + i * head_dim + d] = sum;
                    }
                }
            }
        }
    }

    // Create output tensor
    let mut result = TensorHandle::zeros(&[batch, num_heads, seq_len_q, head_dim], DType::F32)?;
    unsafe {
        let result_data = result.data.as_mut()?;
        let result_ptr = (*result_data.as_ptr()).as_mut_ptr() as *mut f32;
        std::ptr::copy_nonoverlapping(output_data.as_ptr(), result_ptr, total_output);
    }

    Some(result)
}

/// Flash Attention with F64 support.
pub fn flash_attention_cpu_f64(
    query: &TensorHandle,
    key: &TensorHandle,
    value: &TensorHandle,
    scale: f64,
    causal: bool,
) -> Option<TensorHandle> {
    // Validate shapes - expect [batch, num_heads, seq_len, head_dim]
    if query.ndim != 4 || key.ndim != 4 || value.ndim != 4 {
        return None;
    }

    let batch = query.shape[0];
    let num_heads = query.shape[1];
    let seq_len_q = query.shape[2];
    let head_dim = query.shape[3];
    let seq_len_kv = key.shape[2];

    // Validate K and V have consistent shapes
    if key.shape[0] != batch
        || key.shape[1] != num_heads
        || key.shape[3] != head_dim
        || value.shape[0] != batch
        || value.shape[1] != num_heads
        || value.shape[2] != seq_len_kv
        || value.shape[3] != head_dim
    {
        return None;
    }

    if query.dtype != DType::F64 || key.dtype != DType::F64 || value.dtype != DType::F64 {
        return None;
    }

    let q_data = query.data.as_ref()?;
    let k_data = key.data.as_ref()?;
    let v_data = value.data.as_ref()?;

    let total_output = batch * num_heads * seq_len_q * head_dim;
    let mut output_data = vec![0.0f64; total_output];

    unsafe {
        let q_ptr = (*q_data.as_ptr()).as_ptr() as *const f64;
        let k_ptr = (*k_data.as_ptr()).as_ptr() as *const f64;
        let v_ptr = (*v_data.as_ptr()).as_ptr() as *const f64;

        for b in 0..batch {
            for h in 0..num_heads {
                let q_offset = (b * num_heads + h) * seq_len_q * head_dim;
                let k_offset = (b * num_heads + h) * seq_len_kv * head_dim;
                let v_offset = (b * num_heads + h) * seq_len_kv * head_dim;
                let o_offset = (b * num_heads + h) * seq_len_q * head_dim;

                let mut scores = vec![0.0f64; seq_len_q * seq_len_kv];

                // Q @ K^T * scale
                for i in 0..seq_len_q {
                    for j in 0..seq_len_kv {
                        let mut dot = 0.0f64;
                        for d in 0..head_dim {
                            let q_val = *q_ptr.add(q_offset + i * head_dim + d);
                            let k_val = *k_ptr.add(k_offset + j * head_dim + d);
                            dot += q_val * k_val;
                        }
                        scores[i * seq_len_kv + j] = dot * scale;
                    }
                }

                // Causal mask
                if causal {
                    for i in 0..seq_len_q {
                        for j in (i + 1)..seq_len_kv {
                            scores[i * seq_len_kv + j] = f64::NEG_INFINITY;
                        }
                    }
                }

                // Softmax
                for i in 0..seq_len_q {
                    let row_start = i * seq_len_kv;
                    let mut max_val = f64::NEG_INFINITY;
                    for j in 0..seq_len_kv {
                        max_val = max_val.max(scores[row_start + j]);
                    }
                    let mut sum = 0.0f64;
                    for j in 0..seq_len_kv {
                        scores[row_start + j] = (scores[row_start + j] - max_val).exp();
                        sum += scores[row_start + j];
                    }
                    for j in 0..seq_len_kv {
                        scores[row_start + j] /= sum;
                    }
                }

                // Attention @ V
                for i in 0..seq_len_q {
                    for d in 0..head_dim {
                        let mut sum = 0.0f64;
                        for j in 0..seq_len_kv {
                            let attn_weight = scores[i * seq_len_kv + j];
                            let v_val = *v_ptr.add(v_offset + j * head_dim + d);
                            sum += attn_weight * v_val;
                        }
                        output_data[o_offset + i * head_dim + d] = sum;
                    }
                }
            }
        }
    }

    let mut result = TensorHandle::zeros(&[batch, num_heads, seq_len_q, head_dim], DType::F64)?;
    unsafe {
        let result_data = result.data.as_mut()?;
        let result_ptr = (*result_data.as_ptr()).as_mut_ptr() as *mut f64;
        std::ptr::copy_nonoverlapping(output_data.as_ptr(), result_ptr, total_output);
    }

    Some(result)
}

/// Dispatch flash attention based on dtype.
pub fn flash_attention(
    query: &TensorHandle,
    key: &TensorHandle,
    value: &TensorHandle,
    scale: f32,
    causal: bool,
) -> Option<TensorHandle> {
    match query.dtype {
        DType::F32 => flash_attention_cpu(query, key, value, scale, causal),
        DType::F64 => flash_attention_cpu_f64(query, key, value, scale as f64, causal),
        _ => None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dtype_sizes() {
        assert_eq!(DType::F32.size(), 4);
        assert_eq!(DType::F64.size(), 8);
        assert_eq!(DType::I32.size(), 4);
        assert_eq!(DType::I64.size(), 8);
        assert_eq!(DType::Bool.size(), 1);
    }

    #[test]
    fn test_tensor_zeros() {
        let t = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        assert_eq!(t.ndim, 2);
        assert_eq!(t.shape[0], 2);
        assert_eq!(t.shape[1], 3);
        assert_eq!(t.numel, 6);
        assert_eq!(t.dtype, DType::F32);
    }

    #[test]
    fn test_tensor_full() {
        let t = TensorHandle::full(&[2, 2], DType::F32, 3.14).unwrap();
        assert_eq!(t.numel, 4);

        // Check first element
        let val = t.get_scalar_f64();
        assert!(val.is_none()); // Not a scalar tensor (has 4 elements)

        // 0-dimensional scalar tensor
        let scalar = TensorHandle::full(&[1], DType::F64, 42.0).unwrap();
        assert_eq!(scalar.numel, 1);
        let val = scalar.get_scalar_f64().unwrap();
        assert!((val - 42.0).abs() < 1e-10);
    }

    #[test]
    fn test_tensor_binop() {
        let a = TensorHandle::full(&[2, 2], DType::F32, 2.0).unwrap();
        let b = TensorHandle::full(&[2, 2], DType::F32, 3.0).unwrap();

        let c = tensor_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(c.numel, 4);
    }

    #[test]
    fn test_tensor_matmul() {
        let a = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();
        let b = TensorHandle::full(&[3, 2], DType::F32, 1.0).unwrap();

        let c = tensor_matmul(&a, &b).unwrap();
        assert_eq!(c.shape[0], 2);
        assert_eq!(c.shape[1], 2);
        assert_eq!(c.numel, 4);
    }

    #[test]
    fn test_tensor_reduce_sum() {
        let t = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();

        let sum = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
        assert_eq!(sum.ndim, 0);

        // The sum of 6 ones should be 6
        let val = sum.get_scalar_f64().unwrap();
        assert!((val - 6.0).abs() < 1e-6);
    }

    #[test]
    fn test_tensor_reshape() {
        let t = TensorHandle::zeros(&[2, 6], DType::F32).unwrap();
        let r = tensor_reshape(&t, &[3, 4]).unwrap();
        assert_eq!(r.shape[0], 3);
        assert_eq!(r.shape[1], 4);
        assert_eq!(r.numel, 12);
    }

    #[test]
    fn test_tensor_transpose() {
        let t = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let r = tensor_transpose(&t).unwrap();
        assert_eq!(r.shape[0], 3);
        assert_eq!(r.shape[1], 2);
    }

    // ==================== Comprehensive Tests ====================

    #[test]
    fn test_all_dtypes() {
        // Test creating tensors with all supported dtypes
        let dtypes = [
            DType::F32,
            DType::F64,
            DType::F16,
            DType::BF16,
            DType::I32,
            DType::I64,
            DType::I8,
            DType::U8,
            DType::Bool,
        ];

        for dtype in dtypes {
            let t = TensorHandle::zeros(&[2, 2], dtype).unwrap();
            assert_eq!(t.dtype, dtype);
            assert_eq!(t.numel, 4);
        }
    }

    #[test]
    fn test_scalar_tensor() {
        // Scalar tensor (0-dimensional)
        let scalar = TensorHandle::full(&[], DType::F64, 42.0).unwrap();
        assert_eq!(scalar.ndim, 0);
        assert_eq!(scalar.numel, 1);
        assert_eq!(scalar.get_scalar_f64().unwrap(), 42.0);
    }

    #[test]
    fn test_1d_tensor() {
        let t = TensorHandle::full(&[5], DType::F32, 1.0).unwrap();
        assert_eq!(t.ndim, 1);
        assert_eq!(t.shape[0], 5);
        assert_eq!(t.numel, 5);
    }

    #[test]
    fn test_high_dimensional_tensor() {
        let t = TensorHandle::zeros(&[2, 3, 4, 5], DType::F32).unwrap();
        assert_eq!(t.ndim, 4);
        assert_eq!(t.numel, 2 * 3 * 4 * 5);
    }

    #[test]
    fn test_tensor_binop_all_ops() {
        let a = TensorHandle::full(&[2, 2], DType::F32, 6.0).unwrap();
        let b = TensorHandle::full(&[2, 2], DType::F32, 2.0).unwrap();

        // Test all binary operations
        let _ = tensor_binop(&a, &b, TensorBinaryOp::Add).unwrap();
        let _ = tensor_binop(&a, &b, TensorBinaryOp::Sub).unwrap();
        let _ = tensor_binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let _ = tensor_binop(&a, &b, TensorBinaryOp::Div).unwrap();
    }

    #[test]
    fn test_tensor_unop_all_ops() {
        let t = TensorHandle::full(&[2, 2], DType::F32, -3.0).unwrap();

        // Test all unary operations
        let _ = tensor_unop(&t, TensorUnaryOp::Neg).unwrap();
        let _ = tensor_unop(&t, TensorUnaryOp::Abs).unwrap();
    }

    #[test]
    fn test_tensor_reduce_all_ops() {
        let t = TensorHandle::full(&[3, 3], DType::F32, 2.0).unwrap();

        // Test all reduce operations
        let sum = tensor_reduce(&t, None, TensorReduceOp::Sum).unwrap();
        assert_eq!(sum.get_scalar_f64().unwrap(), 18.0);

        let max = tensor_reduce(&t, None, TensorReduceOp::Max).unwrap();
        assert_eq!(max.get_scalar_f64().unwrap(), 2.0);

        let min = tensor_reduce(&t, None, TensorReduceOp::Min).unwrap();
        assert_eq!(min.get_scalar_f64().unwrap(), 2.0);

        let mean = tensor_reduce(&t, None, TensorReduceOp::Mean).unwrap();
        assert_eq!(mean.get_scalar_f64().unwrap(), 2.0);
    }

    #[test]
    fn test_tensor_reduce_axis() {
        let t = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();

        // Sum along axis 0 (rows)
        let sum0 = tensor_reduce(&t, Some(0), TensorReduceOp::Sum).unwrap();
        assert_eq!(sum0.shape[0], 3);

        // Sum along axis 1 (columns)
        let sum1 = tensor_reduce(&t, Some(1), TensorReduceOp::Sum).unwrap();
        assert_eq!(sum1.shape[0], 2);
    }

    #[test]
    fn test_tensor_contiguous() {
        let t = TensorHandle::zeros(&[4, 4], DType::F32).unwrap();
        assert!(t.is_contiguous());
    }

    #[test]
    fn test_tensor_clone() {
        let original = TensorHandle::full(&[2, 2], DType::F32, 5.0).unwrap();
        let cloned = original.clone();

        assert_eq!(original.shape[0], cloned.shape[0]);
        assert_eq!(original.dtype, cloned.dtype);
        assert_eq!(original.numel, cloned.numel);
    }

    #[test]
    fn test_tensor_strides_computation() {
        let t = TensorHandle::zeros(&[2, 3, 4], DType::F32).unwrap();
        // Row-major: strides are in element counts [3*4, 4, 1]
        // (not bytes - byte offset = stride * elem_size)
        assert_eq!(t.strides[0], 12); // 3 * 4
        assert_eq!(t.strides[1], 4);
        assert_eq!(t.strides[2], 1);
    }

    #[test]
    fn test_tensor_empty_shape() {
        // Empty tensor (0 elements)
        let t = TensorHandle::zeros(&[0], DType::F32).unwrap();
        assert_eq!(t.numel, 0);
    }

    #[test]
    fn test_matmul_shapes() {
        // Valid: (2,3) x (3,4) = (2,4)
        let a = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[3, 4], DType::F32).unwrap();
        let c = tensor_matmul(&a, &b).unwrap();
        assert_eq!(c.shape[0], 2);
        assert_eq!(c.shape[1], 4);
    }

    #[test]
    fn test_matmul_invalid_shapes() {
        // Invalid: (2,3) x (4,2) - inner dimensions don't match
        let a = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4, 2], DType::F32).unwrap();
        let result = tensor_matmul(&a, &b);
        assert!(result.is_none());
    }

    #[test]
    fn test_reshape_preserve_numel() {
        let t = TensorHandle::zeros(&[2, 6], DType::F32).unwrap();

        // Valid reshape: 2*6 = 12 = 3*4
        let r = tensor_reshape(&t, &[3, 4]).unwrap();
        assert_eq!(r.numel, t.numel);

        // Valid reshape: 2*6 = 12 = 12*1
        let r2 = tensor_reshape(&t, &[12]).unwrap();
        assert_eq!(r2.numel, t.numel);
    }

    #[test]
    fn test_reshape_invalid() {
        let t = TensorHandle::zeros(&[2, 6], DType::F32).unwrap();

        // Invalid reshape: 2*6 = 12 != 5*3 = 15
        let result = tensor_reshape(&t, &[5, 3]);
        assert!(result.is_none());
    }

    #[test]
    fn test_dtype_conversions() {
        assert!(DType::F32.is_float());
        assert!(DType::F64.is_float());
        assert!(DType::F16.is_float());
        assert!(!DType::I32.is_float());
        assert!(!DType::Bool.is_float());
    }

    #[test]
    fn test_tensor_memory_safety() {
        // Test that tensors properly track reference counts
        let t1 = TensorHandle::zeros(&[10, 10], DType::F32).unwrap();
        let t2 = t1.clone();

        // Both should be valid
        assert_eq!(t1.numel, 100);
        assert_eq!(t2.numel, 100);

        // After drop, memory should be reclaimed (checked via valgrind/miri)
        drop(t1);
        assert_eq!(t2.numel, 100); // t2 should still be valid
    }

    #[test]
    fn test_transpose_identity() {
        // Transpose of transpose should preserve shape
        let t = TensorHandle::zeros(&[3, 5], DType::F32).unwrap();
        let t_t = tensor_transpose(&t).unwrap();
        let t_t_t = tensor_transpose(&t_t).unwrap();

        assert_eq!(t_t_t.shape[0], 3);
        assert_eq!(t_t_t.shape[1], 5);
    }

    #[test]
    fn test_tensor_f64_precision() {
        let t = TensorHandle::full(&[1], DType::F64, std::f64::consts::PI).unwrap();
        let val = t.get_scalar_f64().unwrap();
        assert!((val - std::f64::consts::PI).abs() < 1e-15);
    }

    // ========================================================================
    // Advanced Tensor Operations Tests
    // ========================================================================

    #[test]
    fn test_tensor_dot_1d() {
        // Test 1D dot product: [1, 2, 3] · [4, 5, 6] = 4 + 10 + 18 = 32
        let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
        let b = tensor_from_slice(&[4.0, 5.0, 6.0], &[3], DType::F32).unwrap();

        let result = tensor_dot(&a, &b).unwrap();
        assert_eq!(result.ndim, 0); // Scalar result
        let val = result.get_scalar_f64().unwrap();
        assert!((val - 32.0).abs() < 1e-5, "Expected 32.0, got {}", val);
    }

    #[test]
    fn test_tensor_outer() {
        // Outer product: [1, 2] ⊗ [3, 4, 5] = [[3, 4, 5], [6, 8, 10]]
        let a = tensor_from_slice(&[1.0, 2.0], &[2], DType::F32).unwrap();
        let b = tensor_from_slice(&[3.0, 4.0, 5.0], &[3], DType::F32).unwrap();

        let result = tensor_outer(&a, &b).unwrap();
        assert_eq!(result.shape[0], 2);
        assert_eq!(result.shape[1], 3);
        assert_eq!(result.numel, 6);
    }

    #[test]
    fn test_tensor_batch_matmul() {
        // Batched matmul: [2, 2, 3] @ [2, 3, 2] = [2, 2, 2]
        let a = TensorHandle::full(&[2, 2, 3], DType::F32, 1.0).unwrap();
        let b = TensorHandle::full(&[2, 3, 2], DType::F32, 1.0).unwrap();

        let result = tensor_batch_matmul(&a, &b).unwrap();
        assert_eq!(result.shape[0], 2); // Batch dim
        assert_eq!(result.shape[1], 2); // M
        assert_eq!(result.shape[2], 2); // N
    }

    #[test]
    fn test_tensor_topk() {
        // Create tensor [5, 2, 8, 1, 9, 3] and get top 3
        let t = tensor_from_slice(&[5.0, 2.0, 8.0, 1.0, 9.0, 3.0], &[6], DType::F32).unwrap();

        let (values, indices) = tensor_topk(&t, 3, None, true, true).unwrap();
        assert_eq!(values.shape[0], 3);
        assert_eq!(indices.shape[0], 3);

        // Top 3 largest should be 9, 8, 5
        // Verify values tensor has correct size
        assert_eq!(values.numel, 3);
    }

    #[test]
    fn test_tensor_topk_smallest() {
        // Get smallest 2 values
        let t = tensor_from_slice(&[5.0, 2.0, 8.0, 1.0], &[4], DType::F32).unwrap();

        let (values, _indices) = tensor_topk(&t, 2, None, false, true).unwrap();
        assert_eq!(values.numel, 2);
    }

    #[test]
    fn test_tensor_cumsum() {
        // Cumsum: [1, 2, 3, 4] -> [1, 3, 6, 10]
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::F32).unwrap();

        let result = tensor_cumulative(&t, CumulativeOp::Sum, None).unwrap();
        assert_eq!(result.numel, 4);
    }

    #[test]
    fn test_tensor_cumprod() {
        // Cumprod: [1, 2, 3, 4] -> [1, 2, 6, 24]
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::F32).unwrap();

        let result = tensor_cumulative(&t, CumulativeOp::Prod, None).unwrap();
        assert_eq!(result.numel, 4);
    }

    #[test]
    fn test_tensor_pool2d_max() {
        // Create 1x1x4x4 tensor, apply 2x2 max pooling -> 1x1x2x2
        let t = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 1.0).unwrap();

        let result = tensor_pool2d(&t, PoolOp::Max, (2, 2), (2, 2), (0, 0)).unwrap();
        assert_eq!(result.shape[0], 1);
        assert_eq!(result.shape[1], 1);
        assert_eq!(result.shape[2], 2);
        assert_eq!(result.shape[3], 2);
    }

    #[test]
    fn test_tensor_pool2d_avg() {
        // Average pooling
        let t = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 4.0).unwrap();

        let result = tensor_pool2d(&t, PoolOp::Avg, (2, 2), (2, 2), (0, 0)).unwrap();
        assert_eq!(result.shape[2], 2);
        assert_eq!(result.shape[3], 2);
    }

    #[test]
    fn test_tensor_conv2d_basic() {
        // Simple conv: 1x1x4x4 input, 1x1x3x3 kernel -> 1x1x2x2 output
        let input = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 1.0).unwrap();
        let kernel = TensorHandle::full(&[1, 1, 3, 3], DType::F32, 1.0).unwrap();

        let result = tensor_conv2d(
            &input,
            &kernel,
            None,            // no bias
            (1, 1),          // stride
            (0, 0),          // padding
            (1, 1),          // dilation
            1,               // groups
        ).unwrap();

        assert_eq!(result.shape[0], 1);
        assert_eq!(result.shape[1], 1);
        assert_eq!(result.shape[2], 2); // (4 - 3) / 1 + 1 = 2
        assert_eq!(result.shape[3], 2);
    }

    #[test]
    fn test_tensor_conv2d_with_padding() {
        // Conv with padding to preserve size
        let input = TensorHandle::full(&[1, 1, 4, 4], DType::F32, 1.0).unwrap();
        let kernel = TensorHandle::full(&[1, 1, 3, 3], DType::F32, 1.0).unwrap();

        let result = tensor_conv2d(
            &input,
            &kernel,
            None,
            (1, 1),          // stride
            (1, 1),          // padding
            (1, 1),          // dilation
            1,
        ).unwrap();

        assert_eq!(result.shape[2], 4); // (4 + 2*1 - 3) / 1 + 1 = 4
        assert_eq!(result.shape[3], 4);
    }

    #[test]
    fn test_tensor_cmp_eq() {
        let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
        let b = tensor_from_slice(&[1.0, 5.0, 3.0], &[3], DType::F32).unwrap();

        let result = tensor_cmp(&a, &b, CompareOp::Eq).unwrap();
        assert_eq!(result.dtype, DType::Bool);
        assert_eq!(result.numel, 3);
    }

    #[test]
    fn test_tensor_cmp_lt() {
        let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
        let b = tensor_from_slice(&[2.0, 2.0, 2.0], &[3], DType::F32).unwrap();

        let result = tensor_cmp(&a, &b, CompareOp::Lt).unwrap();
        assert_eq!(result.dtype, DType::Bool);
        assert_eq!(result.numel, 3);
    }

    #[test]
    fn test_broadcast_shapes() {
        // Basic broadcasting
        assert_eq!(broadcast_shapes(&[3, 1], &[1, 4]), Some(vec![3, 4]));
        assert_eq!(broadcast_shapes(&[2, 3, 4], &[3, 4]), Some(vec![2, 3, 4]));
        assert_eq!(broadcast_shapes(&[5], &[5]), Some(vec![5]));

        // Incompatible shapes
        assert_eq!(broadcast_shapes(&[2, 3], &[3, 2]), None);
    }

    #[test]
    fn test_tensor_masked_fill() {
        let src = TensorHandle::full(&[4], DType::F32, 1.0).unwrap();

        // Create boolean mask
        let mask = TensorHandle::zeros(&[4], DType::Bool).unwrap();
        // Set some elements to true - we test that the function works
        // Full mask manipulation would require writing to the tensor

        let result = tensor_masked_fill(&src, &mask, -999.0).unwrap();
        assert_eq!(result.numel, 4);
        assert_eq!(result.dtype, DType::F32);
    }

    #[test]
    fn test_tensor_index_select() {
        // Create source tensor [10, 20, 30, 40, 50]
        let src = tensor_from_slice(&[10.0, 20.0, 30.0, 40.0, 50.0], &[5], DType::F32).unwrap();

        // Create indices tensor [0, 2, 4]
        let indices = TensorHandle::zeros(&[3], DType::I64).unwrap();
        // Note: This test verifies the function shape handling

        let result = tensor_index_select(&src, &indices, 0).unwrap();
        assert_eq!(result.shape[0], 3);
    }

    #[test]
    fn test_tensor_layer_norm() {
        let input = TensorHandle::full(&[2, 4], DType::F32, 1.0).unwrap();

        let result = tensor_layer_norm(&input, None, None, 1e-5).unwrap();
        assert_eq!(result.shape[0], 2);
        assert_eq!(result.shape[1], 4);
    }

    #[test]
    fn test_tensor_batch_norm() {
        let input = TensorHandle::full(&[2, 3, 4, 4], DType::F32, 1.0).unwrap();

        let result = tensor_batch_norm(&input, None, None, None, None, 1e-5, false).unwrap();
        assert_eq!(result.shape[0], 2);
        assert_eq!(result.shape[1], 3);
    }

    #[test]
    fn test_tensor_rms_norm() {
        let input = TensorHandle::full(&[2, 4], DType::F32, 2.0).unwrap();

        let result = tensor_rms_norm(&input, None, 1e-5).unwrap();
        assert_eq!(result.shape[0], 2);
        assert_eq!(result.shape[1], 4);
    }

    #[test]
    fn test_cumulative_ops_2d() {
        // Test cumsum on 2D tensor along axis 0
        let t = TensorHandle::full(&[3, 4], DType::F32, 1.0).unwrap();

        let result = tensor_cumulative(&t, CumulativeOp::Sum, Some(0)).unwrap();
        assert_eq!(result.shape[0], 3);
        assert_eq!(result.shape[1], 4);
    }

    #[test]
    fn test_tensor_conv2d_multi_channel() {
        // Multi-channel conv: 1x3x8x8 input, 16x3x3x3 kernel -> 1x16x6x6
        let input = TensorHandle::full(&[1, 3, 8, 8], DType::F32, 1.0).unwrap();
        let kernel = TensorHandle::full(&[16, 3, 3, 3], DType::F32, 0.1).unwrap();

        let result = tensor_conv2d(
            &input,
            &kernel,
            None,
            (1, 1),
            (0, 0),
            (1, 1),
            1,
        ).unwrap();

        assert_eq!(result.shape[0], 1);
        assert_eq!(result.shape[1], 16);
        assert_eq!(result.shape[2], 6);
        assert_eq!(result.shape[3], 6);
    }

    #[test]
    fn test_tensor_topk_2d() {
        // Test topk on 2D tensor
        let t = TensorHandle::full(&[3, 5], DType::F32, 1.0).unwrap();

        let (values, indices) = tensor_topk(&t, 2, Some(1), true, true).unwrap();
        assert_eq!(values.shape[0], 3);
        assert_eq!(values.shape[1], 2);
        assert_eq!(indices.shape[0], 3);
        assert_eq!(indices.shape[1], 2);
    }

    #[test]
    fn test_tensor_pool2d_strided() {
        // Test pooling with stride > 1
        let t = TensorHandle::full(&[1, 1, 8, 8], DType::F32, 1.0).unwrap();

        let result = tensor_pool2d(&t, PoolOp::Max, (3, 3), (2, 2), (0, 0)).unwrap();
        // Output: ((8 - 3) / 2 + 1) = 3
        assert_eq!(result.shape[2], 3);
        assert_eq!(result.shape[3], 3);
    }

    #[test]
    fn test_broadcast_index() {
        // Test broadcast index computation
        let idx = broadcast_index(5, &[2, 3], &[1, 3]);
        // For shape [2, 3] and src shape [1, 3], index 5 maps to coords [1, 2]
        // Since src dim 0 is 1, coord becomes [0, 2], src_idx = 2
        assert_eq!(idx, 2);
    }

    // ========================================================================
    // Shape Operations Tests
    // ========================================================================

    #[test]
    fn test_tensor_unsqueeze() {
        let t = TensorHandle::zeros(&[3, 4], DType::F32).unwrap();

        // Unsqueeze at dim 0: [3, 4] → [1, 3, 4]
        let r = tensor_unsqueeze(&t, 0).unwrap();
        assert_eq!(r.ndim, 3);
        assert_eq!(r.shape[0], 1);
        assert_eq!(r.shape[1], 3);
        assert_eq!(r.shape[2], 4);

        // Unsqueeze at dim 1: [3, 4] → [3, 1, 4]
        let r = tensor_unsqueeze(&t, 1).unwrap();
        assert_eq!(r.shape[0], 3);
        assert_eq!(r.shape[1], 1);
        assert_eq!(r.shape[2], 4);

        // Unsqueeze at dim -1: [3, 4] → [3, 4, 1]
        let r = tensor_unsqueeze(&t, -1).unwrap();
        assert_eq!(r.shape[0], 3);
        assert_eq!(r.shape[1], 4);
        assert_eq!(r.shape[2], 1);
    }

    #[test]
    fn test_tensor_split() {
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[6], DType::F32).unwrap();

        // Split into 3 equal parts
        let parts = tensor_split(&t, &[3], 0).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].shape[0], 2);
        assert_eq!(parts[1].shape[0], 2);
        assert_eq!(parts[2].shape[0], 2);

        // Split with specific sizes
        let parts = tensor_split(&t, &[1, 2, 3], 0).unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].shape[0], 1);
        assert_eq!(parts[1].shape[0], 2);
        assert_eq!(parts[2].shape[0], 3);
    }

    #[test]
    fn test_tensor_split_at() {
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], &[5], DType::F32).unwrap();

        let (left, right) = tensor_split_at(&t, 2, 0).unwrap();
        assert_eq!(left.shape[0], 2);
        assert_eq!(right.shape[0], 3);
    }

    #[test]
    fn test_tensor_repeat() {
        let t = TensorHandle::full(&[2, 3], DType::F32, 1.0).unwrap();

        // Repeat [2, 3] by [2, 3] → [4, 9]
        let r = tensor_repeat(&t, &[2, 3]).unwrap();
        assert_eq!(r.shape[0], 4);
        assert_eq!(r.shape[1], 9);
        assert_eq!(r.numel, 36);
    }

    #[test]
    fn test_tensor_one_hot() {
        // Create indices tensor with I64 type for the test
        let mut indices = TensorHandle::zeros(&[3], DType::I64).unwrap();
        let ptr = indices.data_ptr_i64_mut();
        unsafe {
            *ptr.add(0) = 0;
            *ptr.add(1) = 2;
            *ptr.add(2) = 1;
        }

        let result = tensor_one_hot(&indices, 4).unwrap();
        assert_eq!(result.shape[0], 3);
        assert_eq!(result.shape[1], 4);
        assert_eq!(result.dtype, DType::F32);

        // Check values
        let ptr = result.data_ptr_f32();
        unsafe {
            // Row 0: [1, 0, 0, 0] (index 0)
            assert_eq!(*ptr.add(0), 1.0);
            assert_eq!(*ptr.add(1), 0.0);
            // Row 1: [0, 0, 1, 0] (index 2)
            assert_eq!(*ptr.add(4), 0.0);
            assert_eq!(*ptr.add(6), 1.0);
            // Row 2: [0, 1, 0, 0] (index 1)
            assert_eq!(*ptr.add(9), 1.0);
        }
    }

    #[test]
    fn test_tensor_flip() {
        // Test flip on 1D
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::F32).unwrap();
        let r = tensor_flip(&t, &[0]).unwrap();

        let ptr = r.data_ptr_f32();
        unsafe {
            assert_eq!(*ptr.add(0), 4.0);
            assert_eq!(*ptr.add(1), 3.0);
            assert_eq!(*ptr.add(2), 2.0);
            assert_eq!(*ptr.add(3), 1.0);
        }
    }

    #[test]
    fn test_tensor_roll() {
        let t = tensor_from_slice(&[1.0, 2.0, 3.0, 4.0], &[4], DType::F32).unwrap();

        // Roll by 1: [1, 2, 3, 4] → [4, 1, 2, 3]
        let r = tensor_roll(&t, 1, 0).unwrap();
        let ptr = r.data_ptr_f32();
        unsafe {
            assert_eq!(*ptr.add(0), 4.0);
            assert_eq!(*ptr.add(1), 1.0);
            assert_eq!(*ptr.add(2), 2.0);
            assert_eq!(*ptr.add(3), 3.0);
        }

        // Roll by -1: [1, 2, 3, 4] → [2, 3, 4, 1]
        let r = tensor_roll(&t, -1, 0).unwrap();
        let ptr = r.data_ptr_f32();
        unsafe {
            assert_eq!(*ptr.add(0), 2.0);
            assert_eq!(*ptr.add(1), 3.0);
            assert_eq!(*ptr.add(2), 4.0);
            assert_eq!(*ptr.add(3), 1.0);
        }
    }

    #[test]
    fn test_tensor_nonzero() {
        // Create tensor with some zeros
        let t = tensor_from_slice(&[0.0, 1.0, 0.0, 2.0, 3.0, 0.0], &[2, 3], DType::F32).unwrap();

        let indices = tensor_nonzero(&t).unwrap();
        // Non-zero values at: (0,1), (1,0), (1,1)
        assert_eq!(indices.shape[0], 3); // 3 non-zero elements
        assert_eq!(indices.shape[1], 2); // 2D indices
    }

    // ========================================================================
    // Boolean Logical Operations Tests
    // ========================================================================

    #[test]
    fn test_tensor_logical_not() {
        let t = tensor_from_bools(&[true, false, true, false], &[4]).unwrap();
        let r = tensor_logical_not(&t).unwrap();

        let vals = tensor_to_bools(&r).unwrap();
        assert_eq!(vals, vec![false, true, false, true]);
    }

    #[test]
    fn test_tensor_logical_and() {
        let a = tensor_from_bools(&[true, true, false, false], &[4]).unwrap();
        let b = tensor_from_bools(&[true, false, true, false], &[4]).unwrap();
        let r = tensor_logical_and(&a, &b).unwrap();

        let vals = tensor_to_bools(&r).unwrap();
        assert_eq!(vals, vec![true, false, false, false]);
    }

    #[test]
    fn test_tensor_logical_or() {
        let a = tensor_from_bools(&[true, true, false, false], &[4]).unwrap();
        let b = tensor_from_bools(&[true, false, true, false], &[4]).unwrap();
        let r = tensor_logical_or(&a, &b).unwrap();

        let vals = tensor_to_bools(&r).unwrap();
        assert_eq!(vals, vec![true, true, true, false]);
    }

    #[test]
    fn test_tensor_logical_xor() {
        let a = tensor_from_bools(&[true, true, false, false], &[4]).unwrap();
        let b = tensor_from_bools(&[true, false, true, false], &[4]).unwrap();
        let r = tensor_logical_xor(&a, &b).unwrap();

        let vals = tensor_to_bools(&r).unwrap();
        assert_eq!(vals, vec![false, true, true, false]);
    }

    #[test]
    fn test_tensor_logical_broadcast() {
        // Test broadcasting: [2,2] AND [2]
        let a = tensor_from_bools(&[true, false, true, true], &[2, 2]).unwrap();
        let b = tensor_from_bools(&[true, false], &[2]).unwrap();
        let r = tensor_logical_and(&a, &b).unwrap();

        assert_eq!(r.shape[0], 2);
        assert_eq!(r.shape[1], 2);

        let vals = tensor_to_bools(&r).unwrap();
        // Row 0: [true AND true, false AND false] = [true, false]
        // Row 1: [true AND true, true AND false] = [true, false]
        assert_eq!(vals, vec![true, false, true, false]);
    }

    #[test]
    fn test_tensor_all() {
        let t_true = tensor_from_bools(&[true, true, true], &[3]).unwrap();
        assert_eq!(tensor_all(&t_true), Some(true));

        let t_false = tensor_from_bools(&[true, false, true], &[3]).unwrap();
        assert_eq!(tensor_all(&t_false), Some(false));

        // Test 2D tensor
        let t_2d = tensor_from_bools(&[true, true, true, true], &[2, 2]).unwrap();
        assert_eq!(tensor_all(&t_2d), Some(true));
    }

    #[test]
    fn test_tensor_any() {
        let t_none = tensor_from_bools(&[false, false, false], &[3]).unwrap();
        assert_eq!(tensor_any(&t_none), Some(false));

        let t_one = tensor_from_bools(&[false, true, false], &[3]).unwrap();
        assert_eq!(tensor_any(&t_one), Some(true));

        // Test 2D tensor
        let t_2d = tensor_from_bools(&[false, false, true, false], &[2, 2]).unwrap();
        assert_eq!(tensor_any(&t_2d), Some(true));
    }

    #[test]
    fn test_tensor_from_bools() {
        let t = tensor_from_bools(&[true, false, true, false, true, false], &[2, 3]).unwrap();
        assert_eq!(t.ndim, 2);
        assert_eq!(t.shape[0], 2);
        assert_eq!(t.shape[1], 3);
        assert_eq!(t.dtype, DType::Bool);
    }

    #[test]
    fn test_tensor_to_bools() {
        let t = tensor_from_bools(&[true, false, true], &[3]).unwrap();
        let vals = tensor_to_bools(&t).unwrap();
        assert_eq!(vals, vec![true, false, true]);
    }

    #[test]
    fn test_tensor_comparison_to_bool() {
        // Create two tensors and compare them
        let a = tensor_from_slice(&[1.0, 2.0, 3.0], &[3], DType::F32).unwrap();
        let b = tensor_from_slice(&[1.0, 3.0, 2.0], &[3], DType::F32).unwrap();

        // Test equality
        let eq = tensor_cmp(&a, &b, CompareOp::Eq).unwrap();
        assert_eq!(eq.dtype, DType::Bool);
        let vals = tensor_to_bools(&eq).unwrap();
        assert_eq!(vals, vec![true, false, false]);

        // Test less than
        let lt = tensor_cmp(&a, &b, CompareOp::Lt).unwrap();
        let vals = tensor_to_bools(&lt).unwrap();
        assert_eq!(vals, vec![false, true, false]);
    }
}
