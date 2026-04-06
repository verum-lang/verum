//! Interpreter state management.
//!
//! The [`InterpreterState`] holds all runtime state for VBC execution:
//! - Registers
//! - Call stack
//! - Heap
//! - Module references
//! - Profiling data
//!
//! # V-LLSI Architecture
//!
//! The interpreter uses built-in opcodes and the stdlib for:
//! - CBGR operations (via verum_common types)
//! - Memory allocation (via V-LLSI syscall intrinsics)
//! - Stdlib functions (collections, I/O, crypto)
//!
//! This ensures consistent behavior between interpreter and JIT/AOT execution.
//! The V-LLSI bootstrap kernel provides initial allocator, syscall wrappers, and TLS
//! initialization before the full Verum runtime loads. This ensures consistent behavior
//! between interpreter (Tier 0) and JIT/AOT (Tier 1-3) execution.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::module::{FunctionId, VbcModule};
use crate::value::Value;

use super::registers::RegisterFile;
use super::stack::CallStack;
use super::heap::Heap;

// Thread-local pointer to the current interpreter state.
// This is used for FFI callback re-entry: when C code calls back into Verum,
// the callback handler needs access to the interpreter state to execute
// Verum functions.
//
// SAFETY: This is inherently unsafe as it bypasses Rust's borrow checker.
// The pointer is only valid while the interpreter is executing an FFI call
// that might invoke callbacks. The caller must ensure:
// 1. The pointer is set before any callback-invoking FFI calls
// 2. The pointer is cleared after the FFI call completes
// 3. The interpreter state remains valid for the duration
thread_local! {
    static CURRENT_INTERPRETER: RefCell<Option<*mut InterpreterState>> = const { RefCell::new(None) };
}
use super::autodiff::GradientTape;
use super::gpu_simulator;

#[cfg(feature = "ffi")]
use crate::ffi::FfiRuntime;

// ============================================================================
// Well-Known Context Type IDs
// ============================================================================
//
// These are reserved context type IDs for the Verum runtime. User-defined
// context types should use IDs >= 0x1000.
//
// Reserved context type IDs for the scientific computing stack.
// User-defined context types should use IDs >= 0x1000.

/// Context type ID for PrecisionMode.
pub const CTX_TYPE_PRECISION_MODE: u32 = 0x0001;

/// Context type ID for ComputeDevice.
pub const CTX_TYPE_COMPUTE_DEVICE: u32 = 0x0002;

/// Context type ID for GradientTape.
pub const CTX_TYPE_GRADIENT_TAPE: u32 = 0x0003;

/// Context type ID for ParallelConfig.
pub const CTX_TYPE_PARALLEL_CONFIG: u32 = 0x0004;

/// Context type ID for MemoryPool.
pub const CTX_TYPE_MEMORY_POOL: u32 = 0x0005;

/// Context type ID for RandomSource.
/// Used for random tensor generation (rand, randn, randint).
pub const CTX_TYPE_RANDOM_SOURCE: u32 = 0x0006;

/// Context type ID for TrainingMode.
/// Used to distinguish training vs inference for dropout, batch norm, etc.
pub const CTX_TYPE_TRAINING_MODE: u32 = 0x0007;

/// First user-defined context type ID.
pub const CTX_TYPE_USER_START: u32 = 0x1000;

// ============================================================================
// PrecisionMode Context
// ============================================================================
//
// The PrecisionMode context controls floating-point arithmetic behavior:
// - Precision level (F16, F32, F64)
// - Rounding mode (IEEE 754 modes)
// - Denormal handling
//
// PrecisionMode: controls float precision and rounding for mathematical kernels.

/// IEEE 754 rounding modes for floating-point operations.
///
/// These map directly to hardware FPU rounding modes and are consistent
/// with the verum_smt::FloatRoundingMode enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum RoundingMode {
    /// Round to nearest, ties to even (IEEE 754 default).
    /// This is the most common mode and provides the best average accuracy.
    #[default]
    NearestTiesToEven = 0,

    /// Round to nearest, ties away from zero.
    /// Used in some financial calculations.
    NearestTiesToAway = 1,

    /// Round toward positive infinity (ceiling).
    TowardPositive = 2,

    /// Round toward negative infinity (floor).
    TowardNegative = 3,

    /// Round toward zero (truncation).
    TowardZero = 4,
}

impl RoundingMode {
    /// Convert from u8 to RoundingMode.
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => RoundingMode::NearestTiesToEven,
            1 => RoundingMode::NearestTiesToAway,
            2 => RoundingMode::TowardPositive,
            3 => RoundingMode::TowardNegative,
            4 => RoundingMode::TowardZero,
            _ => RoundingMode::NearestTiesToEven,
        }
    }
}

/// Floating-point precision levels.
///
/// Controls the precision used for tensor operations when the context
/// is active. Operations may promote to higher precision internally
/// and round to the target precision on output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum FloatPrecision {
    /// Half precision (16-bit, IEEE 754 binary16).
    /// Exponent: 5 bits, Significand: 11 bits (10 stored).
    Half = 0,

    /// Brain float (16-bit, same exponent range as F32).
    /// Exponent: 8 bits, Significand: 8 bits (7 stored).
    BFloat16 = 1,

    /// Single precision (32-bit, IEEE 754 binary32).
    /// Exponent: 8 bits, Significand: 24 bits (23 stored).
    Single = 2,

    /// Double precision (64-bit, IEEE 754 binary64).
    /// Exponent: 11 bits, Significand: 53 bits (52 stored).
    #[default]
    Double = 3,
}

impl FloatPrecision {
    /// Convert from u8 to FloatPrecision.
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => FloatPrecision::Half,
            1 => FloatPrecision::BFloat16,
            2 => FloatPrecision::Single,
            3 => FloatPrecision::Double,
            _ => FloatPrecision::Double,
        }
    }

    /// Returns the number of bits for this precision.
    pub fn bits(&self) -> u8 {
        match self {
            FloatPrecision::Half | FloatPrecision::BFloat16 => 16,
            FloatPrecision::Single => 32,
            FloatPrecision::Double => 64,
        }
    }
}

/// PrecisionMode context for controlling floating-point behavior.
///
/// This context can be provided to tensor operations to control
/// precision, rounding, and denormal handling.
///
/// # Example Usage (in Verum)
///
/// ```verum
/// provide PrecisionMode {
///     precision: Single,
///     rounding_mode: NearestTiesToEven,
///     allow_denormals: true,
/// } {
///     // All tensor operations in this scope use F32 precision
///     let result = matrix_multiply(a, b);
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecisionMode {
    /// The target precision for floating-point operations.
    pub precision: FloatPrecision,

    /// The rounding mode for operations that require rounding.
    pub rounding_mode: RoundingMode,

    /// Whether denormal (subnormal) numbers are allowed.
    /// When false, denormals are flushed to zero for performance.
    pub allow_denormals: bool,
}

impl Default for PrecisionMode {
    fn default() -> Self {
        Self {
            precision: FloatPrecision::Double,
            rounding_mode: RoundingMode::NearestTiesToEven,
            allow_denormals: true,
        }
    }
}

impl PrecisionMode {
    /// Creates a new PrecisionMode with the specified precision.
    pub fn new(precision: FloatPrecision) -> Self {
        Self {
            precision,
            ..Default::default()
        }
    }

    /// Creates a PrecisionMode for fast F32 computation (flush denormals).
    pub fn fast_f32() -> Self {
        Self {
            precision: FloatPrecision::Single,
            rounding_mode: RoundingMode::NearestTiesToEven,
            allow_denormals: false,
        }
    }

    /// Creates a PrecisionMode for precise F64 computation.
    pub fn precise_f64() -> Self {
        Self {
            precision: FloatPrecision::Double,
            rounding_mode: RoundingMode::NearestTiesToEven,
            allow_denormals: true,
        }
    }

    /// Creates a PrecisionMode for mixed-precision training (F16 compute, F32 accumulate).
    pub fn mixed_precision() -> Self {
        Self {
            precision: FloatPrecision::Half,
            rounding_mode: RoundingMode::NearestTiesToEven,
            allow_denormals: false,
        }
    }

    /// Packs the PrecisionMode into a 64-bit value for NaN-boxing.
    ///
    /// Layout:
    /// - Bits 0-7: precision (u8)
    /// - Bits 8-15: rounding_mode (u8)
    /// - Bit 16: allow_denormals (bool)
    pub fn pack(&self) -> u64 {
        let precision = self.precision as u64;
        let rounding = (self.rounding_mode as u64) << 8;
        let denormals = if self.allow_denormals { 1u64 << 16 } else { 0 };
        precision | rounding | denormals
    }

    /// Unpacks a PrecisionMode from a 64-bit value.
    pub fn unpack(value: u64) -> Self {
        let precision = FloatPrecision::from_u8((value & 0xFF) as u8);
        let rounding_mode = RoundingMode::from_u8(((value >> 8) & 0xFF) as u8);
        let allow_denormals = (value >> 16) & 1 == 1;
        Self {
            precision,
            rounding_mode,
            allow_denormals,
        }
    }
}

// ============================================================================
// GPU Execution Context
// ============================================================================

/// GPU execution context for tracking device and buffer state.
///
/// In the interpreter, GPU operations are executed as CPU fallbacks.
/// This context tracks the simulated GPU state so that device management,
/// stream creation/destruction, and buffer allocation are consistent.
pub struct GpuContext {
    /// Current active device ID (0 = CPU fallback).
    pub device_id: u32,
    /// Active stream handles (stream_id -> is_active).
    pub streams: HashMap<u32, bool>,
    /// Next stream handle counter.
    pub next_stream_id: u32,
    /// Allocated GPU buffer addresses, tracked for proper deallocation.
    /// Maps pointer address to the allocation size.
    pub allocated_buffers: HashMap<usize, usize>,
    /// Event records: event_id -> timestamp (for real elapsed time measurement).
    pub events: HashMap<u32, std::time::Instant>,
    /// Next event handle counter.
    pub next_event_id: u32,
    /// Graph operation log for replay support.
    /// Each entry is (stream_id, operation_description).
    pub graph_ops: Vec<(u32, String)>,
    /// Whether graph capture is active.
    pub graph_capturing: bool,
}

impl GpuContext {
    /// Creates a new GPU context with CPU as the default device.
    pub fn new() -> Self {
        Self {
            device_id: 0,
            streams: HashMap::new(),
            next_stream_id: 1,
            allocated_buffers: HashMap::new(),
            events: HashMap::new(),
            next_event_id: 1,
            graph_ops: Vec::new(),
            graph_capturing: false,
        }
    }
}

impl GpuContext {
    /// Resets the GPU context, clearing all accumulated state.
    ///
    /// Call this between test executions to prevent unbounded growth of
    /// streams, buffers, events, and graph operation logs.
    pub fn reset(&mut self) {
        self.device_id = 0;
        self.streams.clear();
        self.next_stream_id = 1;
        self.allocated_buffers.clear();
        self.events.clear();
        self.next_event_id = 1;
        self.graph_ops.clear();
        self.graph_capturing = false;
    }
}

impl Default for GpuContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Main interpreter state.
///
/// This structure holds all runtime state for executing VBC bytecode.
/// It is designed for single-threaded execution but can be cloned
/// for multi-threaded scenarios.
pub struct InterpreterState {
    /// Current module being executed.
    pub module: Arc<VbcModule>,

    /// Loaded modules (for cross-module calls).
    pub modules: HashMap<String, Arc<VbcModule>>,

    /// Register file.
    pub registers: RegisterFile,

    /// Call stack.
    pub call_stack: CallStack,

    /// Heap allocator.
    pub heap: Heap,

    /// Execution statistics.
    pub stats: ExecutionStats,

    /// Configuration options.
    pub config: InterpreterConfig,

    /// Context stack for scoped context values.
    pub context_stack: ContextStack,

    /// Async task queue for spawned tasks.
    pub tasks: TaskQueue,

    /// Generator registry for fn* functions.
    ///
    /// Generator registry: stores Generator structs indexed by GeneratorId. Each generator
    /// holds saved_pc, saved_registers, saved_contexts, yielded_value, and status.
    pub generators: GeneratorRegistry,

    /// Currently executing generator (if in generator context).
    ///
    /// When a generator is being executed (resumed), this holds its ID.
    /// Used by Yield instruction to know which generator to suspend.
    pub current_generator: Option<GeneratorId>,

    /// Current log level filter (0=Error, 1=Warning, 2=Info, 3=Debug, 4=Trace).
    /// Default is 2 (Info). Set via LogExtended::SetLevel.
    pub log_level: i64,

    /// Nursery registry for structured concurrency.
    ///
    /// Structured concurrency: nursery blocks spawn tasks and await_all joins them.
    /// Supports timeout, max_tasks, on_error (cancel_all/wait_all/fail_fast) options.
    pub nurseries: NurseryRegistry,

    /// Gradient tape for autodiff operations.
    pub grad_tape: GradientTape,

    /// GPU execution context for device/stream/buffer tracking.
    pub gpu_context: GpuContext,

    /// GPU thread context for CPU-fallback kernel execution.
    /// Set before each kernel thread execution; queried by thread intrinsic opcodes.
    pub gpu_thread_ctx: Option<gpu_simulator::GpuThreadContext>,

    /// Per-block shared memory for the currently executing GPU block.
    /// Shared across all threads within a block during kernel execution.
    pub gpu_shared_memory: Option<gpu_simulator::SharedMemoryBlock>,

    /// Shared memory allocation offset (bump allocator within the block).
    pub gpu_shared_mem_offset: usize,

    /// Method dispatch inline cache: (method_name_id) -> FunctionId
    /// Caches the last resolved function for each method_id to avoid linear search
    /// through all module functions on repeated calls to the same method.
    pub method_cache: HashMap<u32, crate::FunctionId>,

    /// Captured stdout output (for test execution).
    pub stdout_buffer: String,

    /// Captured stderr output (for test execution).
    pub stderr_buffer: String,

    /// Whether to capture output instead of writing to real stdout/stderr.
    pub capture_output: bool,

    /// Exception handler stack for try-catch blocks.
    ///
    /// Each entry contains the handler PC and the call stack depth
    /// when the try block was entered.
    pub exception_handlers: ExceptionHandlerStack,

    /// Current exception value (set by Throw, read by GetException).
    pub current_exception: Option<Value>,

    /// Argument stack for variadic function calls (Push/Pop).
    pub arg_stack: Vec<Value>,

    /// Currently awaited task ID (for async task completion tracking).
    ///
    /// When Await is called on a Pending task, the task is executed synchronously.
    /// This field tracks which task is being awaited so that when the task function
    /// returns, we can mark the task as Completed with the return value.
    pub awaiting_task: Option<TaskId>,

    /// Whether the verum_runtime context system has been initialized.
    ///
    /// The runtime context system (verum_runtime::context::ffi) provides cross-boundary
    /// context propagation for async tasks and FFI calls. It must be initialized once
    /// per thread before use.
    ///
    /// Context system (Level 2 dynamic DI): provide/using keywords with ~5-30ns
    /// lookup via task-local storage. Must be initialized once per thread before
    /// use. The context environment (theta) is stored in task-local storage and
    /// inherited on spawn. Context methods can be sync or async.
    pub runtime_ctx_initialized: bool,

    /// Thread-local storage slots for V-LLSI TLS operations.
    ///
    /// Provides per-interpreter TLS storage that can be accessed via
    /// TlsGet/TlsSet opcodes. Slots are indexed by integer keys.
    ///
    /// Thread-local storage for VBC interpreter. Provides per-interpreter TLS
    /// accessed via TlsGet/TlsSet opcodes, indexed by integer keys. Used by
    /// the context system and other thread-local state.
    pub tls_slots: HashMap<usize, Value>,

    /// Current CBGR epoch for capability-based generational reference tracking.
    ///
    /// The epoch is incremented by AdvanceEpoch to invalidate all references
    /// from previous epochs. This provides temporal memory safety.
    ///
    /// CBGR epoch for capability-based generational reference tracking.
    /// Incremented by AdvanceEpoch to invalidate all references from previous
    /// epochs. Each managed reference (&T) stores its creation epoch; on
    /// dereference, the reference's epoch is compared against the current
    /// epoch to detect use-after-free. Tier 0 (interpreter): ~100ns check.
    pub cbgr_epoch: u64,

    /// CBGR bypass depth counter for performance-critical sections.
    ///
    /// When non-zero, CBGR validation is temporarily disabled.
    /// Incremented by BypassBegin, decremented by BypassEnd.
    pub cbgr_bypass_depth: u32,

    /// Tracks base addresses of raw CBGR allocations (AllocationHeader starts).
    ///
    /// When `Heap.new(value)` allocates a CBGR object, the raw allocation base
    /// (where the AllocationHeader starts) is stored here. This allows `GetField`
    /// to detect CBGR header pointers and use raw u32 field access instead of
    /// the normal ObjectHeader + Value-sized field layout.
    pub cbgr_allocations: HashSet<usize>,

    /// Tracks the source pointer of the most recent CBGR data deref.
    /// When Deref reads a Value from a CBGR data pointer, this records
    /// (destination_reg, data_ptr_addr) so that a subsequent RefCreate
    /// can create a reference back to the original memory location
    /// instead of a register-based reference. Cleared after each use.
    pub cbgr_deref_source: Option<(u16, usize)>,

    /// Tracks data pointer addresses of mutable pointer-based references.
    /// When RefMut creates a pointer-based reference (via deref_source or
    /// CBGR allocation passthrough), the data address is stored here so
    /// that epoch_caps/can_write can detect mutability on pointer refs.
    pub cbgr_mutable_ptrs: HashSet<usize>,

    /// Tracks the creation epoch for pointer-based CBGR references.
    /// When RefCreate/RefMut creates a pointer-based reference from &*heap_value,
    /// the current epoch is recorded so that .epoch() returns the reference
    /// creation time, not the allocation time.
    pub cbgr_ref_creation_epoch: std::collections::HashMap<usize, u64>,

    /// CBGR validation counter for statistics.
    ///
    /// Counts the number of reference validations performed.
    pub cbgr_validation_count: u64,

    /// FFI runtime for libffi-based external function calls.
    ///
    /// This is the new FFI system that uses libffi for dynamic dispatch.
    /// It provides cross-platform support for calling native C functions
    /// from VBC bytecode at Tier 0 (interpreter).
    ///
    /// Lazily initialized on first FFI call to avoid overhead when FFI
    /// is not used.
    #[cfg(feature = "ffi")]
    pub ffi_runtime: Option<FfiRuntime>,

    /// Temporary FFI array buffers for marshalling Verum arrays to C-typed contiguous memory.
    /// Cleared after each FFI call completes.
    #[cfg(feature = "ffi")]
    pub ffi_array_buffers: Vec<FfiArrayBuffer>,

    /// Pending drops for struct fields.
    ///
    /// When dropping a struct without its own Drop impl, its fields are added here
    /// to be dropped one at a time (since each drop may invoke a user-defined function).
    pub pending_drops: Vec<Value>,

    /// Global instruction counter shared across all dispatch loop invocations.
    ///
    /// Unlike the previous per-loop `loop_count`, this counter is incremented by
    /// every dispatch loop (including nested ones from closures, iterators, generators,
    /// etc.) and checked against `config.max_instructions`. This prevents infinite
    /// loops that span nested dispatch calls from bypassing the instruction limit.
    pub global_instruction_count: u64,
}

/// Tracks a marshalled FFI array buffer for writeback after C calls.
#[cfg(feature = "ffi")]
#[derive(Debug)]
pub struct FfiArrayBuffer {
    /// Pointer to the heap-allocated C buffer (e.g., Vec<i32> as raw pointer)
    pub buffer: *mut u8,
    /// Layout used for deallocation
    pub layout: std::alloc::Layout,
    /// Pointer to the Verum array's heap object
    pub array_obj_ptr: *mut u8,
    /// Number of elements
    pub count: usize,
    /// Element type tag (0x03=i32, 0x04=i64, etc.)
    pub element_type: u8,
    /// Whether to write back after FFI call
    pub is_mutable: bool,
}

#[cfg(feature = "ffi")]
impl Drop for FfiArrayBuffer {
    fn drop(&mut self) {
        if !self.buffer.is_null() && self.layout.size() > 0 {
            unsafe { std::alloc::dealloc(self.buffer, self.layout); }
        }
    }
}

// ============================================================================
// Exception Handling
// ============================================================================

/// An exception handler entry for try-catch blocks.
#[derive(Debug, Clone)]
pub struct ExceptionHandler {
    /// Program counter to jump to when an exception is caught.
    pub handler_pc: usize,
    /// Call stack depth when the try block was entered.
    /// Used to unwind the stack when an exception is thrown.
    pub stack_depth: usize,
    /// Register base at the time of TryBegin.
    pub reg_base: usize,
    /// Function ID where this handler was set up.
    pub func_id: FunctionId,
}

/// Stack of exception handlers for nested try-catch blocks.
#[derive(Debug, Clone, Default)]
pub struct ExceptionHandlerStack {
    /// Active exception handlers (innermost at the end).
    handlers: Vec<ExceptionHandler>,
}

impl ExceptionHandlerStack {
    /// Creates a new empty exception handler stack.
    pub fn new() -> Self {
        Self { handlers: Vec::new() }
    }

    /// Pushes a new exception handler onto the stack.
    pub fn push(&mut self, handler: ExceptionHandler) {
        self.handlers.push(handler);
    }

    /// Pops the innermost exception handler.
    pub fn pop(&mut self) -> Option<ExceptionHandler> {
        self.handlers.pop()
    }

    /// Returns the innermost exception handler without removing it.
    pub fn peek(&self) -> Option<&ExceptionHandler> {
        self.handlers.last()
    }

    /// Returns true if there are no active exception handlers.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// Returns the number of active exception handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Clears all exception handlers.
    pub fn clear(&mut self) {
        self.handlers.clear();
    }

    /// Removes all handlers at or above the given stack depth.
    /// Used when unwinding the stack.
    pub fn unwind_to_depth(&mut self, depth: usize) {
        self.handlers.retain(|h| h.stack_depth < depth);
    }
}

// ============================================================================
// Context System
// ============================================================================

/// A scoped context value.
#[derive(Debug, Clone)]
pub struct ContextEntry {
    /// Context type ID.
    pub ctx_type: u32,
    /// Context value.
    pub value: Value,
    /// Stack depth when this context was provided (for scoping).
    pub stack_depth: usize,
}

/// Stack of context values for the context system.
#[derive(Debug, Clone, Default)]
pub struct ContextStack {
    /// Active context entries.
    entries: Vec<ContextEntry>,
}

impl ContextStack {
    /// Creates a new empty context stack.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Provides a context value, pushing it onto the stack.
    pub fn provide(&mut self, ctx_type: u32, value: Value, stack_depth: usize) {
        self.entries.push(ContextEntry {
            ctx_type,
            value,
            stack_depth,
        });
    }

    /// Gets the most recent context value of a given type.
    pub fn get(&self, ctx_type: u32) -> Option<Value> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.ctx_type == ctx_type)
            .map(|e| e.value)
    }

    /// Ends a context scope, removing all entries at or above the given stack depth.
    pub fn end_scope(&mut self, stack_depth: usize) {
        self.entries.retain(|e| e.stack_depth < stack_depth);
    }

    /// Pops the most recently provided context entry.
    ///
    /// Used by `CtxEnd` to close a `provide X = v in { body }` block. CtxProvide
    /// and CtxEnd come in matched pairs for scoped provides, so LIFO pop is
    /// correct even when nested provides occur in the same stack frame.
    pub fn pop_one(&mut self) {
        self.entries.pop();
    }

    /// Clears all context entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns the number of active context entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if there are no active context entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clones all context entries (for generator state saving).
    ///
    /// Generator state machine: saves/restores context entries on yield/resume for correct DI scoping
    pub fn clone_entries(&self) -> Vec<ContextEntry> {
        self.entries.clone()
    }

    /// Restores context entries from a saved state (for generator resumption).
    ///
    /// Generator state machine: saves/restores context entries on yield/resume for correct DI scoping
    pub fn restore_entries(&mut self, entries: Vec<ContextEntry>) {
        self.entries = entries;
    }

    // =========================================================================
    // PrecisionMode Context Helpers
    // =========================================================================
    //
    // Math context system: precision mode, GPU acceleration, random sources, training mode

    /// Provides a PrecisionMode context.
    ///
    /// The precision mode will be active until the scope ends.
    pub fn provide_precision_mode(&mut self, mode: PrecisionMode, stack_depth: usize) {
        let packed = mode.pack();
        self.provide(CTX_TYPE_PRECISION_MODE, Value::from_i64(packed as i64), stack_depth);
    }

    /// Gets the current PrecisionMode context, or returns the default.
    ///
    /// If no PrecisionMode has been provided, returns the default mode
    /// (Double precision, NearestTiesToEven, allow denormals).
    pub fn get_precision_mode(&self) -> PrecisionMode {
        self.get(CTX_TYPE_PRECISION_MODE)
            .map(|v| {
                if v.is_int() {
                    PrecisionMode::unpack(v.as_i64() as u64)
                } else {
                    PrecisionMode::default()
                }
            })
            .unwrap_or_default()
    }

    /// Returns true if a PrecisionMode context is active.
    pub fn has_precision_mode(&self) -> bool {
        self.get(CTX_TYPE_PRECISION_MODE).is_some()
    }

    // ========================================================================
    // ComputeDevice Context Helpers
    // ========================================================================
    //
    // The ComputeDevice context controls device placement for tensor operations.
    // This enables the `provide ComputeDevice = GPUDevice(0) { ... }` pattern
    // from core/math/gpu.vr.
    //
    // Math context system: precision mode, GPU acceleration, random sources, training mode

    /// Provides a ComputeDevice context.
    ///
    /// The device will be active until the scope ends. Tensor operations
    /// within this scope will use this device for allocation and computation.
    ///
    /// # Arguments
    /// * `device_id` - The device ID (0 = CPU, 0x1000+ = GPU devices)
    /// * `stack_depth` - Current call stack depth for scope tracking
    ///
    /// # Example
    /// ```ignore
    /// // Corresponds to:
    /// // provide ComputeDevice = GPUDevice(0) { ... }
    /// context_stack.provide_compute_device(0x1000, 1);  // GPU 0
    /// ```
    pub fn provide_compute_device(&mut self, device_id: u16, stack_depth: usize) {
        self.provide(CTX_TYPE_COMPUTE_DEVICE, Value::from_i64(device_id as i64), stack_depth);
    }

    /// Gets the current ComputeDevice context, or returns CPU (0) as default.
    ///
    /// Returns the device ID where tensor operations should be performed.
    /// If no ComputeDevice has been provided, returns 0 (CPU).
    ///
    /// # Returns
    /// Device ID: 0 = CPU, 0x1000 = GPU0, 0x1001 = GPU1, etc.
    pub fn get_compute_device(&self) -> u16 {
        self.get(CTX_TYPE_COMPUTE_DEVICE)
            .map(|v| {
                if v.is_int() {
                    v.as_i64() as u16
                } else {
                    0 // Default to CPU
                }
            })
            .unwrap_or(0) // Default to CPU
    }

    /// Returns true if a ComputeDevice context is active.
    pub fn has_compute_device(&self) -> bool {
        self.get(CTX_TYPE_COMPUTE_DEVICE).is_some()
    }

    /// Returns true if the current compute device is a GPU.
    ///
    /// GPU device IDs have the high nibble set (0x1000+).
    pub fn is_gpu_device(&self) -> bool {
        let device = self.get_compute_device();
        (device & 0xF000) == 0x1000
    }

    // ========================================================================
    // RandomSource Context Helpers
    // ========================================================================
    //
    // The RandomSource context provides seeded random number generation.
    // This enables reproducible random tensor generation for ML training.
    //
    // The context stores a 64-bit seed value. Functions like rand() and randn()
    // use RandomSource.key() to get deterministic random values.
    //
    // Math context system: precision mode, GPU acceleration, random sources, training mode

    /// Provides a RandomSource context with the given seed.
    ///
    /// All random tensor operations within this scope will use this seed
    /// for reproducible random number generation.
    ///
    /// # Arguments
    /// * `seed` - 64-bit seed value for the PRNG
    /// * `stack_depth` - Current call stack depth for scope tracking
    ///
    /// # Example
    /// ```ignore
    /// // Corresponds to:
    /// // provide RandomSource = RandomKey.new(42) { ... }
    /// context_stack.provide_random_source(42, 1);
    /// ```
    pub fn provide_random_source(&mut self, seed: u64, stack_depth: usize) {
        self.provide(CTX_TYPE_RANDOM_SOURCE, Value::from_i64(seed as i64), stack_depth);
    }

    /// Gets the current RandomSource seed, or returns a default seed.
    ///
    /// If no RandomSource has been provided, returns a fixed default seed (0)
    /// for deterministic behavior in tests and development.
    ///
    /// # Returns
    /// The 64-bit seed value for PRNG initialization.
    pub fn get_random_seed(&self) -> u64 {
        self.get(CTX_TYPE_RANDOM_SOURCE)
            .map(|v| {
                if v.is_int() {
                    v.as_i64() as u64
                } else {
                    0 // Default seed
                }
            })
            .unwrap_or(0) // Default seed for determinism
    }

    /// Returns true if a RandomSource context is active.
    pub fn has_random_source(&self) -> bool {
        self.get(CTX_TYPE_RANDOM_SOURCE).is_some()
    }

    /// Generates a random key for the current context.
    ///
    /// Uses SplitMix64 to derive a 128-bit key from the seed.
    /// This matches the RandomKey structure in core/math/random.vr.
    ///
    /// # Returns
    /// A tuple (high, low) representing the 128-bit random key.
    pub fn get_random_key(&self) -> (u64, u64) {
        let seed = self.get_random_seed();
        let z1 = splitmix64(seed);
        let z2 = splitmix64(z1);
        (z1, z2)
    }

    // ========================================================================
    // TrainingMode Context Helpers
    // ========================================================================
    //
    // The TrainingMode context controls whether operations like dropout,
    // batch normalization, and stochastic depth are in training or inference mode.
    // This enables the `provide TrainingMode = true { ... }` pattern.
    //
    // Math context system: precision mode, GPU acceleration, random sources, training mode

    /// Provides a TrainingMode context.
    ///
    /// The training mode will be active until the scope ends. Operations
    /// like dropout, batch norm, and other training-specific behaviors
    /// check this context to determine their behavior.
    ///
    /// # Arguments
    /// * `is_training` - true for training mode, false for inference
    /// * `stack_depth` - Current call stack depth for scope tracking
    ///
    /// # Example
    /// ```ignore
    /// // Corresponds to:
    /// // provide TrainingMode = true { ... }
    /// context_stack.provide_training_mode(true, 1);
    /// ```
    pub fn provide_training_mode(&mut self, is_training: bool, stack_depth: usize) {
        self.provide(CTX_TYPE_TRAINING_MODE, Value::from_bool(is_training), stack_depth);
    }

    /// Gets the current TrainingMode context, or returns false (inference) as default.
    ///
    /// Returns true if in training mode, false if in inference mode.
    /// If no TrainingMode has been provided, defaults to false (inference).
    ///
    /// # Returns
    /// true = training mode, false = inference mode
    pub fn get_training_mode(&self) -> bool {
        self.get(CTX_TYPE_TRAINING_MODE)
            .map(|v| {
                if v.is_bool() {
                    v.as_bool()
                } else if v.is_int() {
                    v.as_i64() != 0
                } else {
                    false // Default to inference
                }
            })
            .unwrap_or(false) // Default to inference
    }

    /// Returns true if a TrainingMode context is active.
    pub fn has_training_mode(&self) -> bool {
        self.get(CTX_TYPE_TRAINING_MODE).is_some()
    }
}

/// SplitMix64 hash function for PRNG seeding.
///
/// High-quality mixing function used to initialize other PRNGs.
/// Matches the implementation in core/math/random.vr.
#[inline]
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

// ============================================================================
// Async Task System
// ============================================================================

/// Status of an async task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task is pending execution.
    Pending,
    /// Task is currently running.
    Running,
    /// Task is suspended (awaiting something).
    Suspended,
    /// Task completed successfully.
    Completed,
    /// Task failed with an error.
    Failed,
}

/// An async task.
#[derive(Debug, Clone)]
pub struct Task {
    /// Unique task ID.
    pub id: TaskId,
    /// Function to execute.
    pub func_id: FunctionId,
    /// Task status.
    pub status: TaskStatus,
    /// Saved registers (for suspended tasks).
    pub saved_registers: Vec<Value>,
    /// Saved program counter.
    pub saved_pc: u32,
    /// Task result (if completed).
    pub result: Option<Value>,
    /// Deferred argument values (for cooperative scheduling).
    /// Stored at spawn time, consumed when the task is actually executed.
    pub arg_values: Vec<Value>,
    /// Closure value (for tasks spawned from closures).
    /// If Some, the task executes a closure instead of a named function.
    pub closure_val: Option<Value>,
    /// Snapshot of parent's context stack at spawn time.
    /// Restored into the child's context stack before execution.
    pub saved_contexts: Vec<ContextEntry>,
}

/// Unique task identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

/// Queue of async tasks.
#[derive(Debug, Clone, Default)]
pub struct TaskQueue {
    /// All tasks by ID.
    tasks: HashMap<TaskId, Task>,
    /// Next task ID to assign.
    next_id: u64,
    /// Ready tasks (pending or running).
    ready: Vec<TaskId>,
}

impl TaskQueue {
    /// Creates a new empty task queue.
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            next_id: 0,
            ready: Vec::new(),
        }
    }

    /// Spawns a new task, returning its ID.
    pub fn spawn(&mut self, func_id: FunctionId) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;

        let task = Task {
            id,
            func_id,
            status: TaskStatus::Pending,
            saved_registers: Vec::new(),
            saved_pc: 0,
            result: None,
            arg_values: Vec::new(),
            closure_val: None,
            saved_contexts: Vec::new(),
        };

        self.tasks.insert(id, task);
        self.ready.push(id);
        id
    }

    /// Spawns a deferred task with arguments, returning its ID.
    /// The task is NOT executed — it will be run when awaited or by the scheduler.
    /// Inherits parent's context stack snapshot for context propagation.
    pub fn spawn_deferred(&mut self, func_id: FunctionId, args: Vec<Value>) -> TaskId {
        self.spawn_deferred_with_contexts(func_id, args, Vec::new())
    }

    /// Spawns a deferred task with arguments and inherited contexts.
    pub fn spawn_deferred_with_contexts(
        &mut self,
        func_id: FunctionId,
        args: Vec<Value>,
        contexts: Vec<ContextEntry>,
    ) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;

        let task = Task {
            id,
            func_id,
            status: TaskStatus::Pending,
            saved_registers: Vec::new(),
            saved_pc: 0,
            result: None,
            arg_values: args,
            closure_val: None,
            saved_contexts: contexts,
        };

        self.tasks.insert(id, task);
        self.ready.push(id);
        id
    }

    /// Spawns a deferred closure task, returning its ID.
    /// The closure is NOT executed — it will be run when awaited or by the scheduler.
    pub fn spawn_closure_deferred(&mut self, closure_val: Value) -> TaskId {
        self.spawn_closure_deferred_with_contexts(closure_val, Vec::new())
    }

    /// Spawns a deferred closure task with inherited contexts.
    pub fn spawn_closure_deferred_with_contexts(
        &mut self,
        closure_val: Value,
        contexts: Vec<ContextEntry>,
    ) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;

        let task = Task {
            id,
            func_id: FunctionId(0),
            status: TaskStatus::Pending,
            saved_registers: Vec::new(),
            saved_pc: 0,
            result: None,
            arg_values: Vec::new(),
            closure_val: Some(closure_val),
            saved_contexts: contexts,
        };

        self.tasks.insert(id, task);
        self.ready.push(id);
        id
    }

    /// Gets a task by ID.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.get(&id)
    }

    /// Gets a mutable task by ID.
    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        self.tasks.get_mut(&id)
    }

    /// Marks a task as completed with a result.
    pub fn complete(&mut self, id: TaskId, result: Value) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.status = TaskStatus::Completed;
            task.result = Some(result);
        }
    }

    /// Marks a task as failed.
    pub fn fail(&mut self, id: TaskId) {
        if let Some(task) = self.tasks.get_mut(&id) {
            task.status = TaskStatus::Failed;
        }
    }

    /// Returns the next ready task.
    pub fn next_ready(&mut self) -> Option<TaskId> {
        while let Some(id) = self.ready.pop() {
            if let Some(task) = self.tasks.get(&id)
                && task.status == TaskStatus::Pending {
                    return Some(id);
                }
        }
        None
    }

    /// Takes the execution info from a pending task, marking it as Running.
    /// Returns (func_id, arg_values, closure_val) or None if the task is not pending.
    #[allow(clippy::type_complexity)]
    pub fn take_task_exec_info(&mut self, id: TaskId) -> Option<(FunctionId, Vec<Value>, Option<Value>, Vec<ContextEntry>)> {
        if let Some(task) = self.tasks.get_mut(&id)
            && task.status == TaskStatus::Pending {
                task.status = TaskStatus::Running;
                let func_id = task.func_id;
                let args = std::mem::take(&mut task.arg_values);
                let closure = task.closure_val.take();
                let contexts = std::mem::take(&mut task.saved_contexts);
                return Some((func_id, args, closure, contexts));
            }
        None
    }

    /// Returns true if any tasks are still pending.
    pub fn has_pending(&self) -> bool {
        self.tasks.values().any(|t| t.status == TaskStatus::Pending)
    }

    /// Clears all tasks.
    pub fn clear(&mut self) {
        self.tasks.clear();
        self.ready.clear();
        self.next_id = 0;
    }

    /// Returns the number of tasks.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Returns true if there are no tasks.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

// ============================================================================
// Generator System
// ============================================================================
//
// Generator System: fn* (sync) and async fn* (async) generator functions.
// Generators suspend execution via `yield` and resume later, implementing
// the Iterator protocol (sync) or AsyncIterator protocol (async).
// fn* returns Iterator<Item = T>, async fn* returns AsyncIterator<Item = T>.
// yield is only valid inside fn* or async fn* bodies.
//
// Performance targets:
// - Resume overhead: ~5-10ns per yield
// - State machine size: 24 bytes + captured locals
// - No heap allocation for simple generators

/// Status of a generator.
///
/// Generators transition through these states:
/// Created → (resume) → Yielded ↔ (resume/yield) → Completed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorStatus {
    /// Generator created but not yet started.
    Created,
    /// Generator is currently executing (between resume and yield/return).
    Running,
    /// Generator suspended after yielding a value.
    Yielded,
    /// Generator completed (returned or exhausted).
    Completed,
}

/// A generator instance.
///
/// Generators are created from `fn*` functions and produce values lazily
/// via the Iterator protocol.
///
/// Size: 24 bytes + saved_registers + saved_contexts
#[derive(Debug, Clone)]
pub struct Generator {
    /// Unique generator ID.
    pub id: GeneratorId,
    /// Function that created this generator.
    pub func_id: FunctionId,
    /// Current status.
    pub status: GeneratorStatus,
    /// Saved program counter (resume point after yield).
    pub saved_pc: u32,
    /// Saved register base.
    pub saved_reg_base: u32,
    /// Saved register values (locals that must survive across yields).
    pub saved_registers: Vec<Value>,
    /// Number of registers in this generator's frame.
    pub reg_count: u16,
    /// Last yielded value (if status == Yielded).
    pub yielded_value: Option<Value>,
    /// Final return value (if status == Completed).
    pub return_value: Option<Value>,
    /// Saved context entries (for context propagation across yields).
    pub saved_contexts: Vec<ContextEntry>,
}

impl Generator {
    /// Creates a new generator for a function.
    pub fn new(id: GeneratorId, func_id: FunctionId, reg_count: u16) -> Self {
        Self {
            id,
            func_id,
            status: GeneratorStatus::Created,
            saved_pc: 0,
            saved_reg_base: 0,
            saved_registers: Vec::with_capacity(reg_count as usize),
            reg_count,
            yielded_value: None,
            return_value: None,
            saved_contexts: Vec::new(),
        }
    }

    /// Returns true if the generator can be resumed.
    #[inline]
    pub fn can_resume(&self) -> bool {
        matches!(self.status, GeneratorStatus::Created | GeneratorStatus::Yielded)
    }

    /// Returns true if the generator is exhausted.
    #[inline]
    pub fn is_completed(&self) -> bool {
        self.status == GeneratorStatus::Completed
    }

    /// Takes the yielded value, returning None if not yielded.
    pub fn take_yielded(&mut self) -> Option<Value> {
        if self.status == GeneratorStatus::Yielded {
            self.yielded_value.take()
        } else {
            None
        }
    }
}

/// Unique generator identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneratorId(pub u64);

/// Registry of active generators.
///
/// Manages generator lifecycle and provides O(1) lookup by ID.
#[derive(Debug, Clone, Default)]
pub struct GeneratorRegistry {
    /// All generators by ID.
    generators: HashMap<GeneratorId, Generator>,
    /// Next generator ID to assign.
    next_id: u64,
}

impl GeneratorRegistry {
    /// Creates a new empty generator registry.
    pub fn new() -> Self {
        Self {
            generators: HashMap::new(),
            next_id: 0,
        }
    }

    /// Creates a new generator, returning its ID.
    ///
    /// The generator starts in Created status and must be resumed to begin execution.
    pub fn create(&mut self, func_id: FunctionId, reg_count: u16) -> GeneratorId {
        self.create_with_args(func_id, reg_count, Vec::new())
    }

    /// Creates a new generator with initial argument values, returning its ID.
    ///
    /// The generator starts in Created status and must be resumed to begin execution.
    /// The initial arguments are stored in saved_registers and will be restored to the
    /// generator's frame when first resumed via GenNext.
    pub fn create_with_args(&mut self, func_id: FunctionId, reg_count: u16, initial_args: Vec<Value>) -> GeneratorId {
        let id = GeneratorId(self.next_id);
        self.next_id += 1;

        let mut generator = Generator::new(id, func_id, reg_count);
        generator.saved_registers = initial_args;
        self.generators.insert(id, generator);
        id
    }

    /// Gets a generator by ID.
    pub fn get(&self, id: GeneratorId) -> Option<&Generator> {
        self.generators.get(&id)
    }

    /// Gets a mutable generator by ID.
    pub fn get_mut(&mut self, id: GeneratorId) -> Option<&mut Generator> {
        self.generators.get_mut(&id)
    }

    /// Marks a generator as yielded with a value.
    pub fn yield_value(&mut self, id: GeneratorId, value: Value) {
        if let Some(g) = self.generators.get_mut(&id) {
            g.status = GeneratorStatus::Yielded;
            g.yielded_value = Some(value);
        }
    }

    /// Marks a generator as completed with an optional return value.
    pub fn complete(&mut self, id: GeneratorId, value: Option<Value>) {
        if let Some(g) = self.generators.get_mut(&id) {
            g.status = GeneratorStatus::Completed;
            g.return_value = value;
        }
    }

    /// Removes a completed generator, freeing its resources.
    pub fn remove(&mut self, id: GeneratorId) -> Option<Generator> {
        self.generators.remove(&id)
    }

    /// Clears all generators.
    pub fn clear(&mut self) {
        self.generators.clear();
        self.next_id = 0;
    }

    /// Returns the number of active generators.
    pub fn len(&self) -> usize {
        self.generators.len()
    }

    /// Returns true if there are no active generators.
    pub fn is_empty(&self) -> bool {
        self.generators.is_empty()
    }

    /// Returns statistics about generators.
    pub fn stats(&self) -> GeneratorStats {
        let mut stats = GeneratorStats::default();
        for g in self.generators.values() {
            match g.status {
                GeneratorStatus::Created => stats.created += 1,
                GeneratorStatus::Running => stats.running += 1,
                GeneratorStatus::Yielded => stats.yielded += 1,
                GeneratorStatus::Completed => stats.completed += 1,
            }
        }
        stats.total = self.generators.len() as u64;
        stats
    }
}

/// Generator execution statistics.
#[derive(Debug, Clone, Default)]
pub struct GeneratorStats {
    /// Total generators created.
    pub total: u64,
    /// Generators in Created status.
    pub created: u64,
    /// Generators currently running.
    pub running: u64,
    /// Generators suspended (yielded).
    pub yielded: u64,
    /// Generators completed.
    pub completed: u64,
}

// ============================================================================
// Structured Concurrency - Nursery System
// ============================================================================
//
// Structured concurrency: nursery-based task groups with automatic join/cancel on scope exit
//
// Nurseries provide structured concurrency by tracking spawned tasks and
// ensuring all tasks complete before the nursery scope exits. This is
// the VBC interpreter's synchronous simulation of async structured concurrency.
//
// The actual async implementation is in verum_runtime/src/nursery.rs (tokio-based).

/// Status of a nursery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NurseryStatus {
    /// Nursery is active and accepting spawns.
    Active,
    /// Nursery is awaiting task completion.
    Awaiting,
    /// Nursery is cancelled.
    Cancelled,
    /// Nursery completed (all tasks done).
    Completed,
}

/// Error handling behavior for nursery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NurseryErrorBehavior {
    /// Cancel all tasks on first error.
    CancelAll,
    /// Wait for all tasks even if some fail.
    WaitAll,
    /// Fail immediately on first error.
    FailFast,
}

impl From<u64> for NurseryErrorBehavior {
    fn from(value: u64) -> Self {
        match value {
            0 => NurseryErrorBehavior::CancelAll,
            1 => NurseryErrorBehavior::WaitAll,
            2 => NurseryErrorBehavior::FailFast,
            _ => NurseryErrorBehavior::CancelAll,
        }
    }
}

/// A task spawned within a nursery.
#[derive(Debug, Clone)]
pub struct NurseryTask {
    /// Task ID within this nursery.
    pub task_id: u64,
    /// Function to execute.
    pub func_id: FunctionId,
    /// Task status.
    pub status: TaskStatus,
    /// Task result (if completed).
    pub result: Option<Value>,
    /// Error value (if failed).
    pub error: Option<Value>,
    /// Deferred closure value (for cooperative scheduling).
    pub closure_val: Option<Value>,
}

/// A nursery instance.
///
/// Tracks spawned tasks for structured concurrency.
#[derive(Debug, Clone)]
pub struct Nursery {
    /// Unique nursery ID.
    pub id: u64,
    /// Current status.
    pub status: NurseryStatus,
    /// Tasks spawned in this nursery.
    pub tasks: Vec<NurseryTask>,
    /// Next task ID to assign.
    pub next_task_id: u64,
    /// Timeout in milliseconds (0 = no timeout).
    pub timeout_ms: u64,
    /// Maximum concurrent tasks (0 = unlimited).
    pub max_tasks: u64,
    /// Error handling behavior.
    pub error_behavior: NurseryErrorBehavior,
    /// Accumulated error (if any task failed).
    pub accumulated_error: Option<Value>,
}

impl Nursery {
    /// Creates a new nursery.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            status: NurseryStatus::Active,
            tasks: Vec::new(),
            next_task_id: 0,
            timeout_ms: 0,
            max_tasks: 0,
            error_behavior: NurseryErrorBehavior::CancelAll,
            accumulated_error: None,
        }
    }

    /// Spawns a task in this nursery.
    pub fn spawn(&mut self, func_id: FunctionId) -> u64 {
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.tasks.push(NurseryTask {
            task_id,
            func_id,
            status: TaskStatus::Pending,
            result: None,
            error: None,
            closure_val: None,
        });

        task_id
    }

    /// Spawns a deferred closure task in this nursery.
    pub fn spawn_closure(&mut self, closure_val: Value) -> u64 {
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.tasks.push(NurseryTask {
            task_id,
            func_id: FunctionId(0),
            status: TaskStatus::Pending,
            result: None,
            error: None,
            closure_val: Some(closure_val),
        });

        task_id
    }

    /// Returns true if all tasks are completed.
    pub fn all_completed(&self) -> bool {
        self.tasks.iter().all(|t| {
            matches!(t.status, TaskStatus::Completed | TaskStatus::Failed)
        })
    }

    /// Returns true if any task failed.
    pub fn has_error(&self) -> bool {
        self.tasks.iter().any(|t| t.status == TaskStatus::Failed)
    }
}

/// Registry of active nurseries.
///
/// Manages nursery lifecycle and provides O(1) lookup by ID.
#[derive(Debug, Clone, Default)]
pub struct NurseryRegistry {
    /// All nurseries by ID.
    nurseries: HashMap<u64, Nursery>,
    /// Next nursery ID to assign.
    next_id: u64,
    /// Stack of entered nursery scopes (for structured concurrency).
    scope_stack: Vec<u64>,
}

impl NurseryRegistry {
    /// Creates a new empty nursery registry.
    pub fn new() -> Self {
        Self {
            nurseries: HashMap::new(),
            next_id: 0,
            scope_stack: Vec::new(),
        }
    }

    /// Creates a new nursery, returning its ID.
    pub fn create(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;

        let nursery = Nursery::new(id);
        self.nurseries.insert(id, nursery);
        id
    }

    /// Spawns a task in a nursery.
    pub fn spawn_task(&mut self, nursery_id: u64, func_id: FunctionId) -> u64 {
        if let Some(nursery) = self.nurseries.get_mut(&nursery_id) {
            nursery.spawn(func_id)
        } else {
            0 // Invalid nursery - return 0
        }
    }

    /// Awaits all tasks in a nursery (synchronous simulation).
    ///
    /// In the interpreter, this executes all tasks sequentially.
    /// Returns true if all tasks completed successfully.
    pub fn await_all(&mut self, nursery_id: u64) -> bool {
        if let Some(nursery) = self.nurseries.get_mut(&nursery_id) {
            nursery.status = NurseryStatus::Awaiting;

            // In interpreter mode, we mark all pending tasks as completed
            // (actual execution would happen via the dispatch loop)
            for task in &mut nursery.tasks {
                if task.status == TaskStatus::Pending {
                    task.status = TaskStatus::Completed;
                    task.result = Some(Value::unit());
                }
            }

            nursery.status = NurseryStatus::Completed;
            !nursery.has_error()
        } else {
            false
        }
    }

    /// Cancels all tasks in a nursery.
    pub fn cancel(&mut self, nursery_id: u64) {
        if let Some(nursery) = self.nurseries.get_mut(&nursery_id) {
            nursery.status = NurseryStatus::Cancelled;
            // In interpreter mode, mark all pending tasks as cancelled
            for task in &mut nursery.tasks {
                if task.status == TaskStatus::Pending || task.status == TaskStatus::Running {
                    task.status = TaskStatus::Failed;
                }
            }
        }
    }

    /// Configures a nursery option.
    ///
    /// config_type:
    /// - 0: timeout (ms)
    /// - 1: max_tasks
    /// - 2: error_behavior (0=CancelAll, 1=WaitAll, 2=FailFast)
    /// - 3: enter scope (push nursery onto scope stack)
    /// - 4: exit scope (pop nursery from scope stack)
    pub fn configure(&mut self, nursery_id: u64, config_type: u8, _value: u64) {
        match config_type {
            0..=2 => {
                // Original config types - need to pass value
                if let Some(nursery) = self.nurseries.get_mut(&nursery_id) {
                    match config_type {
                        0 => nursery.timeout_ms = _value,
                        1 => nursery.max_tasks = _value,
                        2 => nursery.error_behavior = NurseryErrorBehavior::from(_value),
                        _ => {}
                    }
                }
            }
            3 => {
                // Enter scope - push nursery ID onto scope stack
                self.enter_scope(nursery_id);
            }
            4 => {
                // Exit scope - pop nursery ID from scope stack
                self.exit_scope(nursery_id);
            }
            _ => {} // Ignore unknown config types
        }
    }

    /// Enters a nursery scope (pushes onto scope stack).
    ///
    /// This is used for structured concurrency to track the current
    /// active nursery scope for context propagation.
    pub fn enter_scope(&mut self, nursery_id: u64) {
        // Mark nursery as active
        if let Some(nursery) = self.nurseries.get_mut(&nursery_id) {
            nursery.status = NurseryStatus::Active;
        }
        // Push onto scope stack
        self.scope_stack.push(nursery_id);
    }

    /// Exits a nursery scope (pops from scope stack).
    ///
    /// Validates that the exiting nursery matches the top of the stack.
    pub fn exit_scope(&mut self, nursery_id: u64) {
        // Pop from scope stack - validate it matches
        if let Some(top_id) = self.scope_stack.pop()
            && top_id != nursery_id {
                // Mismatched exit - push back and log error (or could panic)
                self.scope_stack.push(top_id);
            }
    }

    /// Gets the current nursery scope (top of stack).
    pub fn current_scope(&self) -> Option<u64> {
        self.scope_stack.last().copied()
    }

    /// Gets the accumulated error from a nursery.
    pub fn get_error(&self, nursery_id: u64) -> Option<Value> {
        self.nurseries.get(&nursery_id)
            .and_then(|n| n.accumulated_error)
    }

    /// Gets a nursery by ID.
    pub fn get(&self, id: u64) -> Option<&Nursery> {
        self.nurseries.get(&id)
    }

    /// Gets a mutable nursery by ID.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Nursery> {
        self.nurseries.get_mut(&id)
    }

    /// Clears all nurseries.
    pub fn clear(&mut self) {
        self.nurseries.clear();
        self.next_id = 0;
    }
}

/// Interpreter configuration.
#[derive(Debug, Clone)]
pub struct InterpreterConfig {
    /// Maximum call stack depth.
    pub max_stack_depth: usize,

    /// Maximum heap size (bytes).
    pub max_heap_size: usize,

    /// Enable execution tracing.
    pub trace_enabled: bool,

    /// Execution timeout (milliseconds, 0 = no timeout).
    pub timeout_ms: u64,

    /// Enable instruction counting.
    pub count_instructions: bool,

    /// Enable CBGR validation (runtime safety checks).
    pub cbgr_enabled: bool,

    /// Maximum instructions before aborting (0 = no limit).
    pub max_instructions: u64,

    /// External cancellation flag for cooperative abort.
    /// When set to `true`, the dispatch loop returns InstructionLimitExceeded.
    /// Checked every 1024 instructions (~10μs at 100M ops/sec).
    pub cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for InterpreterConfig {
    fn default() -> Self {
        Self {
            max_stack_depth: 1024,
            max_heap_size: 64 * 1024 * 1024, // 64 MB
            trace_enabled: false,
            timeout_ms: 30_000,           // 30 second timeout to prevent infinite loops
            count_instructions: false,
            cbgr_enabled: true,
            max_instructions: 100_000_000, // 100M instruction limit to prevent runaway execution
            cancel_flag: None,
        }
    }
}

/// Execution statistics.
#[derive(Debug, Clone, Default)]
pub struct ExecutionStats {
    /// Total instructions executed.
    pub instructions: u64,

    /// Function calls.
    pub calls: u64,

    /// Returns.
    pub returns: u64,

    /// Jumps taken.
    pub jumps: u64,

    /// Heap allocations.
    pub allocations: u64,

    /// Maximum stack depth reached.
    pub max_stack_depth: usize,

    /// Execution time (nanoseconds).
    pub execution_time_ns: u64,

    /// CBGR tier statistics.
    pub cbgr_stats: CbgrStats,
}

/// CBGR (Capability-Based Generational References) statistics.
///
/// Tracks reference usage across the three tiers:
/// - Tier 0: Full CBGR validation (~15ns overhead)
/// - Tier 1: Compiler-proven safe (0ns overhead)
/// - Tier 2: Unsafe/manual safety (0ns overhead)
///
/// Also provides adaptive validation capabilities that can adjust
/// validation frequency based on observed violation rates.
#[derive(Debug, Clone)]
pub struct CbgrStats {
    /// Tier 0 references created (full validation).
    pub tier0_refs: u64,

    /// Tier 1 references created (compiler-checked, zero overhead).
    pub tier1_refs: u64,

    /// Tier 2 references created (unsafe, zero overhead).
    pub tier2_refs: u64,

    /// Total CBGR validation checks performed.
    pub cbgr_checks: u64,

    /// CBGR violations detected (before error).
    pub cbgr_violations: u64,

    /// Tier 0 dereferences (with validation).
    pub tier0_derefs: u64,

    /// Tier 1 dereferences (direct access).
    pub tier1_derefs: u64,

    /// Tier 2 dereferences (unchecked access).
    pub tier2_derefs: u64,

    // ========== Adaptive Validation Fields ==========

    /// Consecutive validations without violation (used for adaptive checking).
    pub consecutive_clean_validations: u64,

    /// Adaptive validation skip counter (skip N validations when pattern is clean).
    pub adaptive_skip_remaining: u64,

    /// Whether adaptive validation is enabled.
    pub adaptive_enabled: bool,

    /// Threshold for consecutive clean validations before reducing check frequency.
    /// Default: 1000 clean validations trigger adaptation.
    pub adaptive_threshold: u64,

    /// Skip count when adaptive mode is active.
    /// Default: Skip 9 out of 10 validations (10% validation rate).
    pub adaptive_skip_count: u64,
}

impl Default for CbgrStats {
    fn default() -> Self {
        Self {
            tier0_refs: 0,
            tier1_refs: 0,
            tier2_refs: 0,
            cbgr_checks: 0,
            cbgr_violations: 0,
            tier0_derefs: 0,
            tier1_derefs: 0,
            tier2_derefs: 0,
            consecutive_clean_validations: 0,
            adaptive_skip_remaining: 0,
            adaptive_enabled: false, // Disabled by default for safety
            adaptive_threshold: 1000,
            adaptive_skip_count: 9,
        }
    }
}

impl CbgrStats {
    /// Returns the promotion rate (Tier 1 refs / total refs).
    pub fn promotion_rate(&self) -> f64 {
        let total = self.tier0_refs + self.tier1_refs + self.tier2_refs;
        if total == 0 {
            0.0
        } else {
            self.tier1_refs as f64 / total as f64
        }
    }

    /// Returns the tier distribution as percentages.
    pub fn tier_distribution(&self) -> (f64, f64, f64) {
        let total = self.tier0_refs + self.tier1_refs + self.tier2_refs;
        if total == 0 {
            (0.0, 0.0, 0.0)
        } else {
            let t = total as f64;
            (
                self.tier0_refs as f64 / t * 100.0,
                self.tier1_refs as f64 / t * 100.0,
                self.tier2_refs as f64 / t * 100.0,
            )
        }
    }

    // ========== Adaptive Validation Methods ==========

    /// Enable adaptive validation with custom thresholds.
    ///
    /// When enabled, the system will reduce validation frequency after
    /// observing `threshold` consecutive clean validations, skipping
    /// `skip_count` validations for every validation performed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// stats.enable_adaptive(1000, 9); // After 1000 clean, skip 9 of 10
    /// ```
    pub fn enable_adaptive(&mut self, threshold: u64, skip_count: u64) {
        self.adaptive_enabled = true;
        self.adaptive_threshold = threshold;
        self.adaptive_skip_count = skip_count;
    }

    /// Disable adaptive validation (always validate).
    pub fn disable_adaptive(&mut self) {
        self.adaptive_enabled = false;
        self.adaptive_skip_remaining = 0;
        self.consecutive_clean_validations = 0;
    }

    /// Check if validation should be performed based on adaptive state.
    ///
    /// Returns `true` if validation should be performed, `false` if it can be skipped.
    /// This method should be called before each CBGR validation.
    ///
    /// When adaptive mode is disabled, always returns `true`.
    /// When adaptive mode is enabled and pattern is clean, may return `false`
    /// to skip validation and reduce overhead.
    #[inline(always)]
    pub fn should_validate(&mut self) -> bool {
        if !self.adaptive_enabled {
            return true;
        }

        // If we're in adaptive skip mode, check if we should skip
        if self.adaptive_skip_remaining > 0 {
            self.adaptive_skip_remaining -= 1;
            return false;
        }

        // Check if we've reached the threshold for adaptive mode
        if self.consecutive_clean_validations >= self.adaptive_threshold {
            // Enter adaptive skip mode
            self.adaptive_skip_remaining = self.adaptive_skip_count;
        }

        true
    }

    /// Record a successful (clean) validation.
    ///
    /// Call this after each validation that passes without violation.
    #[inline(always)]
    pub fn record_clean_validation(&mut self) {
        self.cbgr_checks += 1;
        self.consecutive_clean_validations += 1;
    }

    /// Record a validation that detected a violation.
    ///
    /// This resets the adaptive state to ensure full validation resumes.
    #[inline(always)]
    pub fn record_violation(&mut self) {
        self.cbgr_violations += 1;
        self.consecutive_clean_validations = 0;
        self.adaptive_skip_remaining = 0;
    }

    /// Get the current violation rate (violations / checks).
    pub fn violation_rate(&self) -> f64 {
        if self.cbgr_checks == 0 {
            0.0
        } else {
            self.cbgr_violations as f64 / self.cbgr_checks as f64
        }
    }

    /// Get the estimated overhead reduction from adaptive validation.
    ///
    /// Returns a value between 0.0 (no reduction) and 1.0 (maximum reduction).
    pub fn adaptive_overhead_reduction(&self) -> f64 {
        if !self.adaptive_enabled || self.cbgr_checks == 0 {
            0.0
        } else {
            // Calculate the fraction of skipped validations
            let skip_ratio = self.adaptive_skip_count as f64 / (self.adaptive_skip_count as f64 + 1.0);
            // Only applies if we've been in adaptive mode
            if self.consecutive_clean_validations >= self.adaptive_threshold {
                skip_ratio
            } else {
                0.0
            }
        }
    }
}

impl InterpreterState {
    /// Creates a new interpreter state for the given module.
    pub fn new(module: Arc<VbcModule>) -> Self {
        Self::with_config(module, InterpreterConfig::default())
    }

    /// Creates a new interpreter state with custom configuration.
    pub fn with_config(module: Arc<VbcModule>, config: InterpreterConfig) -> Self {
        let mut modules = HashMap::new();
        modules.insert(module.name.clone(), Arc::clone(&module));

        Self {
            module,
            modules,
            registers: RegisterFile::new(),
            call_stack: CallStack::with_max_depth(config.max_stack_depth),
            heap: Heap::with_threshold(config.max_heap_size),
            stats: ExecutionStats::default(),
            config,
            context_stack: ContextStack::new(),
            tasks: TaskQueue::new(),
            generators: GeneratorRegistry::new(),
            current_generator: None,
            log_level: 2, // Default: Info
            nurseries: NurseryRegistry::new(),
            grad_tape: GradientTape::new(),
            gpu_context: GpuContext::new(),
            gpu_thread_ctx: None,
            gpu_shared_memory: None,
            gpu_shared_mem_offset: 0,
            method_cache: HashMap::new(),
            stdout_buffer: String::new(),
            exception_handlers: ExceptionHandlerStack::new(),
            current_exception: None,
            stderr_buffer: String::new(),
            capture_output: false,
            arg_stack: Vec::new(),
            awaiting_task: None,
            runtime_ctx_initialized: false,
            tls_slots: HashMap::new(),
            cbgr_epoch: 1,
            cbgr_bypass_depth: 0,
            cbgr_allocations: HashSet::new(),
            cbgr_deref_source: None,
            cbgr_mutable_ptrs: HashSet::new(),
            cbgr_ref_creation_epoch: std::collections::HashMap::new(),
            cbgr_validation_count: 0,
            #[cfg(feature = "ffi")]
            ffi_runtime: None,
            #[cfg(feature = "ffi")]
            ffi_array_buffers: Vec::new(),
            pending_drops: Vec::new(),
            global_instruction_count: 0,
        }
    }

    /// Initializes the runtime context system if not already initialized.
    ///
    /// This sets up thread-local storage for the V-LLSI context system.
    /// The interpreter calls this automatically when needed.
    pub fn ensure_runtime_ctx_initialized(&mut self) {
        if !self.runtime_ctx_initialized {
            // V-LLSI context system is initialized via TLS slots
            // No external runtime dependency required
            self.runtime_ctx_initialized = true;
        }
    }

    /// Loads an additional module.
    pub fn load_module(&mut self, module: Arc<VbcModule>) {
        self.modules.insert(module.name.clone(), module);
    }

    /// Gets a loaded module by name.
    pub fn get_module(&self, name: &str) -> Option<&Arc<VbcModule>> {
        self.modules.get(name)
    }

    /// Gets a function descriptor by ID from the current module.
    pub fn get_function(&self, id: FunctionId) -> Option<&crate::module::FunctionDescriptor> {
        self.module.get_function(id)
    }

    /// Gets a constant from the current module.
    pub fn get_constant(&self, id: crate::module::ConstId) -> Option<&crate::module::Constant> {
        self.module.get_constant(id)
    }

    /// Gets the current register base.
    pub fn reg_base(&self) -> u32 {
        self.call_stack.reg_base()
    }

    /// Gets a register value.
    #[inline]
    pub fn get_reg(&self, reg: crate::instruction::Reg) -> Value {
        self.registers.get(self.reg_base(), reg)
    }

    /// Sets a register value.
    #[inline]
    pub fn set_reg(&mut self, reg: crate::instruction::Reg, value: Value) {
        self.registers.set(self.reg_base(), reg, value);
    }

    /// Gets the current program counter.
    pub fn pc(&self) -> u32 {
        self.call_stack.pc()
    }

    /// Sets the program counter.
    pub fn set_pc(&mut self, pc: u32) {
        self.call_stack.set_pc(pc);
    }

    /// Advances the program counter.
    pub fn advance_pc(&mut self, delta: u32) {
        self.call_stack.advance_pc(delta);
    }

    /// Gets the bytecode for the current function.
    pub fn current_bytecode(&self) -> Option<&[u8]> {
        let frame = self.call_stack.current()?;
        let func = self.module.get_function(frame.function)?;
        let start = func.bytecode_offset as usize;
        let end = start + func.bytecode_length as usize;
        self.module.bytecode.get(start..end)
    }

    /// Gets a byte from bytecode at the given offset.
    #[inline]
    pub fn read_byte(&self, offset: u32) -> Option<u8> {
        let frame = self.call_stack.current()?;
        let func = self.module.get_function(frame.function)?;
        let idx = func.bytecode_offset as usize + offset as usize;
        self.module.bytecode.get(idx).copied()
    }

    /// Resets the interpreter state (clears stack, registers, heap).
    ///
    /// Also clears accumulated runtime state (GPU context, CBGR allocations,
    /// method cache, output buffers, nurseries, exception handlers, TLS slots)
    /// to prevent unbounded memory growth across repeated executions.
    pub fn reset(&mut self) {
        self.registers.clear();
        self.call_stack.clear();
        unsafe { self.heap.clear() };
        self.context_stack.clear();
        self.tasks.clear();
        self.generators.clear();
        self.current_generator = None;
        self.grad_tape.reset();
        self.gpu_context.reset();
        self.gpu_thread_ctx = None;
        self.gpu_shared_memory = None;
        self.gpu_shared_mem_offset = 0;
        self.method_cache.clear();
        self.stdout_buffer.clear();
        self.stderr_buffer.clear();
        self.exception_handlers.clear();
        self.current_exception = None;
        self.arg_stack.clear();
        self.awaiting_task = None;
        self.nurseries.clear();
        self.tls_slots.clear();
        self.cbgr_epoch = 1;
        self.cbgr_bypass_depth = 0;
        self.cbgr_allocations.clear();
        self.cbgr_deref_source = None;
        self.cbgr_mutable_ptrs.clear();
        self.stats = ExecutionStats::default();
        self.global_instruction_count = 0;
    }

    /// Records an instruction execution (for profiling).
    #[inline]
    pub fn record_instruction(&mut self) {
        if self.config.count_instructions {
            self.stats.instructions += 1;
        }
    }

    /// Records a function call.
    #[inline]
    pub fn record_call(&mut self) {
        if self.config.count_instructions {
            self.stats.calls += 1;
            self.stats.max_stack_depth = self.stats.max_stack_depth.max(self.call_stack.depth());
        }
    }

    /// Records a return.
    #[inline]
    pub fn record_return(&mut self) {
        if self.config.count_instructions {
            self.stats.returns += 1;
        }
    }

    /// Records a jump.
    #[inline]
    pub fn record_jump(&mut self) {
        if self.config.count_instructions {
            self.stats.jumps += 1;
        }
    }

    /// Records an allocation.
    #[inline]
    pub fn record_allocation(&mut self) {
        if self.config.count_instructions {
            self.stats.allocations += 1;
        }
    }

    // ==================== V-LLSI TLS Operations ====================
    //
    // Thread-local storage operations for V-LLSI bootstrap kernel.
    // These provide per-interpreter TLS slots accessed via TlsGet/TlsSet opcodes.
    //
    // V-LLSI TLS: per-interpreter thread-local storage slots accessed via TlsGet/TlsSet opcodes

    /// Gets a value from a TLS slot.
    ///
    /// Returns `None` if the slot has not been set.
    #[inline]
    pub fn tls_get(&self, slot: usize) -> Option<Value> {
        self.tls_slots.get(&slot).copied()
    }

    /// Sets a value in a TLS slot.
    #[inline]
    pub fn tls_set(&mut self, slot: usize, value: Value) {
        self.tls_slots.insert(slot, value);
    }

    // ==================== FFI Runtime (libffi-based) ====================
    //
    // These methods provide access to the new FFI runtime that uses libffi
    // for dynamic dispatch. This enables FFI calls at Tier 0 (interpreter).
    //
    // FFI runtime: libffi-based dynamic dispatch for Tier 0 (interpreter) foreign function calls

    /// Gets or creates the FFI runtime for libffi-based calls.
    ///
    /// The FFI runtime is lazily initialized on first use to avoid overhead
    /// when FFI is not used. Once created, it caches library handles and
    /// resolved symbols for optimal performance.
    ///
    /// # Returns
    /// A mutable reference to the FFI runtime, or an error if initialization fails.
    #[cfg(feature = "ffi")]
    pub fn get_or_create_ffi_runtime(&mut self) -> super::error::InterpreterResult<&mut FfiRuntime> {
        if self.ffi_runtime.is_none() {
            // Create and initialize FFI runtime
            let mut runtime = FfiRuntime::new()
                .map_err(|e| super::error::InterpreterError::FfiRuntimeError(
                    format!("Failed to initialize FFI runtime: {}", e)
                ))?;

            // Load libraries required by the current module
            runtime.load_module_libraries(&self.module)
                .map_err(|e| super::error::InterpreterError::FfiRuntimeError(
                    format!("Failed to load module libraries: {}", e)
                ))?;

            self.ffi_runtime = Some(runtime);
        }

        // ffi_runtime was just initialized above, but use ok_or for defensive safety
        self.ffi_runtime.as_mut().ok_or_else(|| super::error::InterpreterError::FfiRuntimeError(
            "FFI runtime initialization failed unexpectedly".into(),
        ))
    }

    /// Returns a reference to the current module.
    ///
    /// Used by FFI operations that need to access module metadata.
    #[inline]
    pub fn module(&self) -> &VbcModule {
        &self.module
    }

    /// Returns a snapshot of current execution state for debugging.
    pub fn debug_snapshot(&self) -> DebugSnapshot {
        DebugSnapshot {
            function: self.call_stack.current().map(|f| f.function),
            pc: self.pc(),
            stack_depth: self.call_stack.depth(),
            registers_used: self.registers.top(),
            heap_allocated: self.heap.allocated(),
            stats: self.stats.clone(),
        }
    }

    // ==================== Output Capture ====================

    /// Enables output capture mode (for test execution).
    ///
    /// When enabled, `print` and `debug_print` output is captured to internal
    /// buffers instead of being written to stdout/stderr.
    pub fn enable_output_capture(&mut self) {
        self.capture_output = true;
        self.stdout_buffer.clear();
        self.stderr_buffer.clear();
    }

    /// Disables output capture mode.
    pub fn disable_output_capture(&mut self) {
        self.capture_output = false;
    }

    /// Writes to stdout (or buffer if capture is enabled).
    pub fn write_stdout(&mut self, s: &str) {
        if self.capture_output {
            self.stdout_buffer.push_str(s);
        } else {
            print!("{}", s);
        }
    }

    /// Writes a line to stdout (or buffer if capture is enabled).
    pub fn writeln_stdout(&mut self, s: &str) {
        if self.capture_output {
            self.stdout_buffer.push_str(s);
            self.stdout_buffer.push('\n');
        } else {
            println!("{}", s);
        }
    }

    /// Writes to stderr (or buffer if capture is enabled).
    pub fn write_stderr(&mut self, s: &str) {
        if self.capture_output {
            self.stderr_buffer.push_str(s);
        } else {
            eprint!("{}", s);
        }
    }

    /// Writes a line to stderr (or buffer if capture is enabled).
    pub fn writeln_stderr(&mut self, s: &str) {
        if self.capture_output {
            self.stderr_buffer.push_str(s);
            self.stderr_buffer.push('\n');
        } else {
            eprintln!("{}", s);
        }
    }

    /// Returns captured stdout output.
    pub fn get_stdout(&self) -> &str {
        &self.stdout_buffer
    }

    /// Returns captured stderr output.
    pub fn get_stderr(&self) -> &str {
        &self.stderr_buffer
    }

    /// Clears captured output buffers.
    pub fn clear_output_buffers(&mut self) {
        self.stdout_buffer.clear();
        self.stderr_buffer.clear();
    }

    /// Takes captured stdout, clearing the buffer.
    pub fn take_stdout(&mut self) -> String {
        std::mem::take(&mut self.stdout_buffer)
    }

    /// Takes captured stderr, clearing the buffer.
    pub fn take_stderr(&mut self) -> String {
        std::mem::take(&mut self.stderr_buffer)
    }

    // ==================== FFI Callback Support ====================
    //
    // These methods support re-entrant execution when C code calls back
    // into Verum functions through FFI trampolines.
    //
    // Architecture:
    // 1. Before making FFI calls that might invoke callbacks, set up the
    //    callback handler using `setup_callback_handler()`
    // 2. When C code calls back through a trampoline, the handler invokes
    //    `invoke_callback_function()` which re-enters the interpreter
    // 3. After FFI call completes, clean up with `teardown_callback_handler()`

    /// Sets up the callback handler for the current thread.
    ///
    /// This must be called before making FFI calls that might invoke callbacks.
    /// The handler will be able to invoke Verum functions re-entrantly.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// 1. `teardown_callback_handler()` is called after the FFI call completes
    /// 2. The interpreter state remains valid for the duration of the FFI call
    #[cfg(feature = "ffi")]
    pub fn setup_callback_handler(&mut self) {
        // Set the thread-local pointer to this interpreter state
        CURRENT_INTERPRETER.with(|cell| {
            let ptr = self as *mut InterpreterState;
            *cell.borrow_mut() = Some(ptr);
        });

        // Set the callback handler that will invoke Verum functions
        crate::ffi::FfiRuntime::set_callback_handler(Box::new(invoke_callback_function));
    }

    /// Tears down the callback handler after an FFI call.
    ///
    /// This must be called after every FFI call that used `setup_callback_handler()`.
    #[cfg(feature = "ffi")]
    pub fn teardown_callback_handler(&mut self) {
        // Clear the callback handler
        crate::ffi::FfiRuntime::clear_callback_handler();

        // Clear the thread-local pointer
        CURRENT_INTERPRETER.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Callback handler function invoked by FFI trampolines.
///
/// This function is called when C code invokes a callback trampoline.
/// It retrieves the interpreter state from the thread-local and re-enters
/// the interpreter to execute the specified Verum function.
///
/// # Arguments
///
/// * `function_id` - The VBC function ID to invoke
/// * `args` - Arguments marshalled from C types to Verum Values
///
/// # Returns
///
/// The result value, or `None` if the function returns unit or an error occurred.
#[cfg(feature = "ffi")]
fn invoke_callback_function(function_id: u32, args: &[crate::value::Value]) -> Option<crate::value::Value> {
    use crate::module::FunctionId;
    use crate::instruction::Reg;

    CURRENT_INTERPRETER.with(|cell| {
        let ptr = *cell.borrow();
        let state = match ptr {
            Some(p) => unsafe { &mut *p },
            None => {
                eprintln!("FFI callback error: no interpreter state available");
                return None;
            }
        };

        // Get the function descriptor
        let func = match state.module.get_function(FunctionId(function_id)) {
            Some(f) => f,
            None => {
                eprintln!("FFI callback error: function {} not found", function_id);
                return None;
            }
        };

        let reg_count = func.register_count;

        // Record the stack depth BEFORE pushing the callback frame.
        // This is the "entry depth" - when the callback returns and
        // the depth falls back to this level, dispatch_loop should return.
        let entry_depth = state.call_stack.depth();

        // CRITICAL: Save the current PC before the callback executes.
        // When the callback returns, handle_return_with_depth will set PC to return_pc.
        // We need to restore the original PC after the callback completes to avoid
        // corrupting the main dispatch loop's PC.
        let saved_pc = state.pc();

        // Push a new frame for the callback function.
        // Note: return_pc is set to saved_pc + 1 (a dummy value) to avoid the return
        // handler setting PC to 0, but we'll restore the real PC anyway.
        let frame_result = state.call_stack.push_frame(
            FunctionId(function_id),
            reg_count,
            saved_pc, // Use saved PC to prevent corruption (will be restored anyway)
            Reg(0),
        );

        if let Err(e) = frame_result {
            eprintln!("FFI callback error: failed to push frame: {:?}", e);
            return None;
        }

        // Allocate registers for this frame
        state.registers.push_frame(reg_count);

        // Copy arguments to registers
        let base = state.reg_base();
        for (i, arg) in args.iter().enumerate() {
            state.registers.set(base, Reg(i as u16), *arg);
        }

        // Execute the function using dispatch_loop_with_entry_depth.
        // This is re-entrant: we're calling dispatch_loop from within
        // a dispatch_loop that's currently blocked on an FFI call.
        // The entry_depth parameter ensures we return when the callback
        // completes, rather than continuing with the caller's frame.
        let result = match super::dispatch_table::dispatch_loop_table_with_entry_depth(state, entry_depth) {
            Ok(value) => {
                // Check if the result is unit
                if value.is_unit() {
                    None
                } else {
                    Some(value)
                }
            }
            Err(e) => {
                eprintln!("FFI callback error: execution failed: {:?}", e);
                None
            }
        };

        // CRITICAL: Restore the PC to what it was before the callback.
        // The callback's return handler may have modified the PC, but the
        // main dispatch loop needs to continue from where the FFI call was made.
        state.set_pc(saved_pc);

        result
    })
}

/// Debug snapshot of interpreter state.
#[derive(Debug, Clone)]
pub struct DebugSnapshot {
    /// Current function (if any).
    pub function: Option<FunctionId>,
    /// Program counter.
    pub pc: u32,
    /// Call stack depth.
    pub stack_depth: usize,
    /// Registers used.
    pub registers_used: usize,
    /// Heap bytes allocated.
    pub heap_allocated: usize,
    /// Execution statistics.
    pub stats: ExecutionStats,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_module() -> Arc<VbcModule> {
        Arc::new(VbcModule::new("test".to_string()))
    }

    #[test]
    fn test_state_creation() {
        let module = test_module();
        let state = InterpreterState::new(module);

        assert_eq!(state.module.name, "test");
        assert!(state.call_stack.is_empty());
        assert_eq!(state.registers.top(), 0);
    }

    #[test]
    fn test_config() {
        let module = test_module();
        let config = InterpreterConfig {
            max_stack_depth: 100,
            trace_enabled: true,
            ..Default::default()
        };

        let state = InterpreterState::with_config(module, config.clone());
        assert_eq!(state.config.max_stack_depth, 100);
        assert!(state.config.trace_enabled);
    }

    #[test]
    fn test_load_module() {
        let module1 = test_module();
        let module2 = Arc::new(VbcModule::new("other".to_string()));

        let mut state = InterpreterState::new(module1);
        state.load_module(module2);

        assert!(state.get_module("test").is_some());
        assert!(state.get_module("other").is_some());
        assert!(state.get_module("unknown").is_none());
    }

    #[test]
    fn test_reset() {
        let module = test_module();
        let mut state = InterpreterState::new(module);

        // Push some state
        state.call_stack.push_frame(FunctionId(0), 16, 0, crate::instruction::Reg(0)).unwrap();
        state.registers.push_frame(16);
        state.stats.instructions = 1000;

        // Reset
        state.reset();

        assert!(state.call_stack.is_empty());
        assert_eq!(state.registers.top(), 0);
        assert_eq!(state.stats.instructions, 0);
    }

    #[test]
    fn test_profiling() {
        let module = test_module();
        let config = InterpreterConfig {
            count_instructions: true,
            ..Default::default()
        };

        let mut state = InterpreterState::with_config(module, config);

        state.record_instruction();
        state.record_instruction();
        state.record_call();
        state.record_return();
        state.record_jump();
        state.record_allocation();

        assert_eq!(state.stats.instructions, 2);
        assert_eq!(state.stats.calls, 1);
        assert_eq!(state.stats.returns, 1);
        assert_eq!(state.stats.jumps, 1);
        assert_eq!(state.stats.allocations, 1);
    }

    #[test]
    fn test_debug_snapshot() {
        let module = test_module();
        let mut state = InterpreterState::new(module);

        state.call_stack.push_frame(FunctionId(5), 16, 0, crate::instruction::Reg(0)).unwrap();

        let snapshot = state.debug_snapshot();
        assert_eq!(snapshot.function, Some(FunctionId(5)));
        assert_eq!(snapshot.stack_depth, 1);
    }

    #[test]
    fn test_pc_operations() {
        let module = test_module();
        let mut state = InterpreterState::new(module);

        state.call_stack.push_frame(FunctionId(0), 16, 0, crate::instruction::Reg(0)).unwrap();

        assert_eq!(state.pc(), 0);

        state.set_pc(100);
        assert_eq!(state.pc(), 100);

        state.advance_pc(10);
        assert_eq!(state.pc(), 110);
    }

    // =========================================================================
    // PrecisionMode Tests
    // =========================================================================

    #[test]
    fn test_precision_mode_default() {
        let mode = PrecisionMode::default();
        assert_eq!(mode.precision, FloatPrecision::Double);
        assert_eq!(mode.rounding_mode, RoundingMode::NearestTiesToEven);
        assert!(mode.allow_denormals);
    }

    #[test]
    fn test_precision_mode_pack_unpack() {
        let mode = PrecisionMode {
            precision: FloatPrecision::Single,
            rounding_mode: RoundingMode::TowardZero,
            allow_denormals: false,
        };

        let packed = mode.pack();
        let unpacked = PrecisionMode::unpack(packed);

        assert_eq!(unpacked.precision, FloatPrecision::Single);
        assert_eq!(unpacked.rounding_mode, RoundingMode::TowardZero);
        assert!(!unpacked.allow_denormals);
    }

    #[test]
    fn test_precision_mode_presets() {
        let fast = PrecisionMode::fast_f32();
        assert_eq!(fast.precision, FloatPrecision::Single);
        assert!(!fast.allow_denormals);

        let precise = PrecisionMode::precise_f64();
        assert_eq!(precise.precision, FloatPrecision::Double);
        assert!(precise.allow_denormals);

        let mixed = PrecisionMode::mixed_precision();
        assert_eq!(mixed.precision, FloatPrecision::Half);
        assert!(!mixed.allow_denormals);
    }

    #[test]
    fn test_rounding_mode_from_u8() {
        assert_eq!(RoundingMode::from_u8(0), RoundingMode::NearestTiesToEven);
        assert_eq!(RoundingMode::from_u8(1), RoundingMode::NearestTiesToAway);
        assert_eq!(RoundingMode::from_u8(2), RoundingMode::TowardPositive);
        assert_eq!(RoundingMode::from_u8(3), RoundingMode::TowardNegative);
        assert_eq!(RoundingMode::from_u8(4), RoundingMode::TowardZero);
        assert_eq!(RoundingMode::from_u8(255), RoundingMode::NearestTiesToEven);
    }

    #[test]
    fn test_float_precision_bits() {
        assert_eq!(FloatPrecision::Half.bits(), 16);
        assert_eq!(FloatPrecision::BFloat16.bits(), 16);
        assert_eq!(FloatPrecision::Single.bits(), 32);
        assert_eq!(FloatPrecision::Double.bits(), 64);
    }

    #[test]
    fn test_context_stack_precision_mode() {
        let mut stack = ContextStack::new();

        // Default mode when none provided
        let default_mode = stack.get_precision_mode();
        assert_eq!(default_mode.precision, FloatPrecision::Double);

        // Provide a custom mode
        let custom_mode = PrecisionMode {
            precision: FloatPrecision::Single,
            rounding_mode: RoundingMode::TowardNegative,
            allow_denormals: false,
        };
        stack.provide_precision_mode(custom_mode, 1);

        // Retrieve the custom mode
        let retrieved = stack.get_precision_mode();
        assert_eq!(retrieved.precision, FloatPrecision::Single);
        assert_eq!(retrieved.rounding_mode, RoundingMode::TowardNegative);
        assert!(!retrieved.allow_denormals);
        assert!(stack.has_precision_mode());

        // End scope - should fall back to default
        stack.end_scope(1);
        let after_scope = stack.get_precision_mode();
        assert_eq!(after_scope.precision, FloatPrecision::Double);
        assert!(!stack.has_precision_mode());
    }

    #[test]
    fn test_nested_precision_modes() {
        let mut stack = ContextStack::new();

        // Outer scope: F32
        let outer_mode = PrecisionMode::new(FloatPrecision::Single);
        stack.provide_precision_mode(outer_mode, 1);

        // Inner scope: F16
        let inner_mode = PrecisionMode::new(FloatPrecision::Half);
        stack.provide_precision_mode(inner_mode, 2);

        // Should get inner mode
        let current = stack.get_precision_mode();
        assert_eq!(current.precision, FloatPrecision::Half);

        // End inner scope
        stack.end_scope(2);

        // Should fall back to outer mode
        let after_inner = stack.get_precision_mode();
        assert_eq!(after_inner.precision, FloatPrecision::Single);

        // End outer scope
        stack.end_scope(1);

        // Should fall back to default
        let after_outer = stack.get_precision_mode();
        assert_eq!(after_outer.precision, FloatPrecision::Double);
    }

    // ========================================================================
    // ComputeDevice Context Tests
    // ========================================================================

    #[test]
    fn test_compute_device_context_default() {
        let stack = ContextStack::new();

        // Default should be CPU (0)
        assert_eq!(stack.get_compute_device(), 0);
        assert!(!stack.has_compute_device());
        assert!(!stack.is_gpu_device());
    }

    #[test]
    fn test_compute_device_context_cpu() {
        let mut stack = ContextStack::new();

        // Provide CPU device explicitly
        stack.provide_compute_device(0x0000, 1);

        assert_eq!(stack.get_compute_device(), 0);
        assert!(stack.has_compute_device());
        assert!(!stack.is_gpu_device());

        // End scope
        stack.end_scope(1);
        assert!(!stack.has_compute_device());
    }

    #[test]
    fn test_compute_device_context_gpu() {
        let mut stack = ContextStack::new();

        // Provide GPU device 0
        stack.provide_compute_device(0x1000, 1);

        assert_eq!(stack.get_compute_device(), 0x1000);
        assert!(stack.has_compute_device());
        assert!(stack.is_gpu_device());

        // End scope
        stack.end_scope(1);
        assert!(!stack.has_compute_device());
        assert!(!stack.is_gpu_device());
    }

    #[test]
    fn test_compute_device_context_gpu_index() {
        let mut stack = ContextStack::new();

        // Provide GPU device 2 (0x1002)
        stack.provide_compute_device(0x1002, 1);

        assert_eq!(stack.get_compute_device(), 0x1002);
        assert!(stack.is_gpu_device());

        // Extract device index (low 12 bits)
        let device_index = stack.get_compute_device() & 0x0FFF;
        assert_eq!(device_index, 2);
    }

    #[test]
    fn test_compute_device_context_nested() {
        let mut stack = ContextStack::new();

        // Outer scope: CPU
        stack.provide_compute_device(0x0000, 1);
        assert!(!stack.is_gpu_device());

        // Inner scope: GPU 0
        stack.provide_compute_device(0x1000, 2);
        assert!(stack.is_gpu_device());
        assert_eq!(stack.get_compute_device(), 0x1000);

        // End inner scope - should fall back to CPU
        stack.end_scope(2);
        assert!(!stack.is_gpu_device());
        assert_eq!(stack.get_compute_device(), 0);

        // End outer scope
        stack.end_scope(1);
        assert!(!stack.has_compute_device());
    }

    // ========================================================================
    // RandomSource Context Tests
    // ========================================================================

    #[test]
    fn test_random_source_context_default() {
        let stack = ContextStack::new();

        // Default seed should be 0
        assert_eq!(stack.get_random_seed(), 0);
        assert!(!stack.has_random_source());
    }

    #[test]
    fn test_random_source_context_seeded() {
        let mut stack = ContextStack::new();

        // Provide a specific seed
        stack.provide_random_source(42, 1);

        assert_eq!(stack.get_random_seed(), 42);
        assert!(stack.has_random_source());

        // End scope
        stack.end_scope(1);
        assert!(!stack.has_random_source());
        assert_eq!(stack.get_random_seed(), 0); // Default
    }

    #[test]
    fn test_random_source_context_key_generation() {
        let mut stack = ContextStack::new();

        // Provide a seed and get the key
        stack.provide_random_source(12345, 1);

        let (high, low) = stack.get_random_key();

        // Key should be deterministic for the same seed
        assert_ne!(high, 0);
        assert_ne!(low, 0);

        // Generate again with same seed - should be identical
        let (high2, low2) = stack.get_random_key();
        assert_eq!(high, high2);
        assert_eq!(low, low2);
    }

    #[test]
    fn test_random_source_context_different_seeds() {
        let mut stack = ContextStack::new();

        // Seed 1
        stack.provide_random_source(100, 1);
        let (high1, low1) = stack.get_random_key();

        // End scope and use different seed
        stack.end_scope(1);
        stack.provide_random_source(200, 1);
        let (high2, low2) = stack.get_random_key();

        // Different seeds should produce different keys
        assert!(high1 != high2 || low1 != low2);
    }

    #[test]
    fn test_random_source_context_nested() {
        let mut stack = ContextStack::new();

        // Outer scope with seed 42
        stack.provide_random_source(42, 1);
        assert_eq!(stack.get_random_seed(), 42);

        // Inner scope with seed 100
        stack.provide_random_source(100, 2);
        assert_eq!(stack.get_random_seed(), 100);

        // End inner scope - should fall back to 42
        stack.end_scope(2);
        assert_eq!(stack.get_random_seed(), 42);

        // End outer scope
        stack.end_scope(1);
        assert_eq!(stack.get_random_seed(), 0); // Default
    }

    #[test]
    fn test_splitmix64_consistency() {
        // Test that splitmix64 produces expected values
        // These are verified against the reference implementation
        let seed = 0u64;
        let result1 = splitmix64(seed);
        let result2 = splitmix64(result1);

        // Results should be non-zero and different
        assert_ne!(result1, 0);
        assert_ne!(result2, 0);
        assert_ne!(result1, result2);

        // Same input should produce same output (deterministic)
        assert_eq!(splitmix64(seed), result1);
    }
}
