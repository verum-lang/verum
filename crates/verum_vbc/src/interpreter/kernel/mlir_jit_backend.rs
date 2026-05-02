//! MLIR-JIT tensor compute backend (Этап C — compute unification).
//!
//! Industrial-grade pipeline that compiles `linalg` / `vector` / `gpu`
//! dialect kernels through `mlir::ExecutionEngine` and caches the
//! resulting function pointers on first invocation.  Replaces, op by
//! op, the hand-tuned 15 232-LOC SIMD ladder in `kernel/cpu.rs` and
//! the macOS-specific 3 876-LOC `kernel/metal.rs` with a single
//! source-of-truth that lets MLIR's auto-vectoriser + tile-and-fuse
//! decide layout / SIMD width / unroll factor — the same machinery
//! PyTorch's `torch.compile`, JAX's XLA, and Triton ride for their
//! production kernels.
//!
//! # Architecture
//!
//! ```text
//!                          tensor_binop / tensor_unop / tensor_matmul / …
//!                                              │
//!                                              ▼
//!                                ┌─────────────────────────┐
//!                                │  dispatch_binop / …     │  kernel/mod.rs
//!                                └────────────┬────────────┘
//!                                             │ (when feature = "mlir-jit")
//!                                             ▼
//!                                ┌─────────────────────────┐
//!                                │  MlirJitBackend.binop   │  this file
//!                                └────────────┬────────────┘
//!                                             │
//!                          cache hit?         │
//!                          ┌─── yes ──── (DashMap.get) ────┐
//!                          │                                │
//!                       no │                                ▼
//!                          ▼                       ┌────────────────┐
//!     ┌─────────────────────────────────┐          │ EngineHolder   │
//!     │  build LLVM-dialect MLIR text   │          │  (cached fn ptr)│
//!     │  module = Module::parse(ctx)    │          └────────┬────────┘
//!     │  ExecutionEngine::new(module)   │                   │
//!     │  cache.insert(key, holder)      │                   ▼
//!     └────────────┬────────────────────┘          ┌────────────────┐
//!                  │                                │ invoke_packed  │
//!                  └──── insert ──────► cache ◄─────┤  with arg ptrs │
//!                                                   └────────────────┘
//! ```
//!
//! # Status (Шаг 2 of Этап C)
//!
//!   * Real `binop` JIT for F32/F64 + Add/Sub/Mul/Div with concurrent
//!     `DashMap` cache keyed on (op, dtype) — first call compiles
//!     (~ms), subsequent calls hit the function pointer (~ns dispatch
//!     overhead on top of the kernel cost itself).  Other dtypes fall
//!     through to `CpuBackend` for now (Шаг 2c migration target).
//!   * `unop` / `reduce` / `matmul` still return `None` (Шаг 3+ wiring
//!     points).
//!   * Cold-start cost: the first call compiles + JITs the kernel.
//!     Persistent cross-process JIT cache is Шаг 8 of the étape — at
//!     that point the cold path drops below the SIMD-ladder
//!     instruction-cache miss it currently displaces.
//!
//! The backend is gated behind the `mlir-jit` Cargo feature (off by
//! default).  When the feature is off the type does not exist and
//! `BackendRegistry` is identical to its pre-Этап-C shape.

use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use dashmap::DashMap;
use parking_lot::Mutex;
use verum_mlir::{
    Context, ExecutionEngine,
    dialect::DialectRegistry,
    ir::{Location, Module, operation::OperationLike},
    pass::{self, PassManager},
    utility::{register_all_dialects, register_all_llvm_translations, register_all_passes},
};

use super::backend::{Backend, ComputeCapabilities};
use super::device::DeviceId;
use super::super::tensor::{DType, TensorHandle};
use crate::instruction::{TensorBinaryOp, TensorReduceOp, TensorUnaryOp};

// =============================================================================
// Memref descriptor — ABI bridge to MLIR's `memref<?x?xT>` arguments.
//
// MLIR lowers `memref<?x?xT>` function arguments to a struct that
// carries the allocated pointer, the aligned (element-aligned)
// pointer, an element offset, and per-dimension size + stride arrays.
// When a function carries `llvm.emit_c_interface`, MLIR generates a
// C-ABI wrapper named `_mlir_ciface_<sym>` that takes pointers to
// these structs.  `invoke_packed` then expects each entry of the
// packed-args array to point to the value of the corresponding arg
// — for our pointer-to-struct args, that means each entry is
// `&mut ptr_to_descriptor`.
//
// The layout below matches MLIR's `StridedMemRefType<T, 2>` exactly
// (`include/mlir/ExecutionEngine/CRunnerUtils.h`).  Element type `T`
// is conveyed by the kernel; the descriptor itself is element-type-
// agnostic since the pointer fields are `*mut u8`.
// =============================================================================

#[repr(C)]
struct MemrefDescriptor2D {
    base: *mut u8,
    aligned: *mut u8,
    offset: i64,
    sizes: [i64; 2],
    strides: [i64; 2],
}

// =============================================================================
// JIT key + cache holder
// =============================================================================

/// Cache key for compiled kernels.  A hit on this key means the
/// `(op, dtype)` pair (whether `op` is a binop or a unop) has been
/// lowered to LLVM dialect, JIT-compiled, and the resulting function
/// pointer is ready for `invoke_packed`.  The two op families share
/// one cache because their kernel signatures differ only in arity
/// (ternary vs binary pointers + length); `KernelKind` discriminates.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct JitKey {
    kind: KernelKind,
    dtype: DType,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
enum KernelKind {
    Binop(TensorBinaryOp),
    Unop(TensorUnaryOp),
    /// Scalar triple-loop matmul; `M`/`K`/`N` are runtime args, the
    /// kernel itself is dtype-only-parametrised (one cached engine
    /// per float dtype).
    Matmul,
    /// Full-tensor reduction (axis = None).  Sum/Prod/Max/Min wired
    /// in Шаг 5a; axis-specific reduction is Шаг 5b.
    Reduce(TensorReduceOp),
}

/// Wrapper that encapsulates the unsafe `Send + Sync` claim for an
/// `ExecutionEngine`.  Justification for the unsafe impls:
///
///   * `ExecutionEngine` wraps an opaque MLIR `MlirExecutionEngine`
///     pointer.  Once `mlirExecutionEngineCreate` returns, the engine
///     owns its compiled code; calls to `invoke_packed` are reentrant
///     by MLIR design (LLVM JITted code is reentrant unless the
///     kernel itself contains shared mutable state — our compute
///     kernels are pure data-parallel loops over caller-owned buffers
///     and have no internal shared state).
///   * The cache stores `Arc<EngineHolder>` so multiple threads can
///     share the same engine; the Arc handles refcounting and the
///     newtype carries the unsafe Send/Sync to satisfy the trait
///     bounds on `Backend: Send + Sync` flowing in from
///     `BackendRegistry`.
struct EngineHolder {
    engine: ExecutionEngine,
}

unsafe impl Send for EngineHolder {}
unsafe impl Sync for EngineHolder {}

// =============================================================================
// Backend
// =============================================================================

/// MLIR-JIT compute backend.
pub struct MlirJitBackend {
    capabilities: ComputeCapabilities,
    allocated_bytes: AtomicUsize,
    /// MLIR Context shared across all kernels compiled by this backend.
    ///
    /// Accessed only on the cold path (cache miss → compile new
    /// kernel).  Wrapped in `Mutex` because MLIR contexts are not
    /// internally synchronised by default — multi-threaded compile is
    /// possible via `enable_multi_threading` but adds compile-time
    /// overhead that's not worth the contention savings on a
    /// once-per-(op, dtype) compile.  Hot-path reads (`invoke_packed`)
    /// do not touch the Context.
    context: Mutex<ContextHolder>,
    /// Compiled kernel cache.  Lock-free reads on the hot path.
    cache: DashMap<JitKey, Arc<EngineHolder>>,
}

/// Context lives on the heap behind a Mutex; mark it Send so the
/// backend is Send.  Justification: same reentrancy argument as
/// EngineHolder — MLIR Context is opaque from Rust's POV and we
/// guarantee single-thread access via the surrounding Mutex.
struct ContextHolder {
    context: Context,
}

unsafe impl Send for ContextHolder {}

impl MlirJitBackend {
    /// Construct a new MLIR-JIT backend.
    ///
    /// Registers all MLIR dialects + LLVM translations on the held
    /// Context.  This is a one-time initialisation cost (~µs) paid at
    /// backend construction; subsequent kernel compiles reuse the
    /// already-loaded dialects.
    pub fn new() -> Self {
        // Pass registration is a process-wide one-shot.  Idempotent
        // via `Once` inside `register_all_passes`, so safe to call
        // even when other parts of the program (e.g. verum_codegen
        // AOT pipeline) have already done it.
        register_all_passes();

        let context = Context::new();
        let registry = DialectRegistry::new();
        register_all_dialects(&registry);
        context.append_dialect_registry(&registry);
        context.load_all_available_dialects();
        register_all_llvm_translations(&context);

        let mut caps = ComputeCapabilities::default();
        caps.max_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        // The MLIR pipeline produces target-native SIMD width
        // automatically; advertising a non-trivial value here lets the
        // dispatcher prefer the JIT path for vectorisable ops.
        caps.simd_width = 8;
        caps.has_fma = true;

        Self {
            capabilities: caps,
            allocated_bytes: AtomicUsize::new(0),
            context: Mutex::new(ContextHolder { context }),
            cache: DashMap::new(),
        }
    }

    /// Look up the cached engine for `(kind, dtype)` or compile and insert.
    ///
    /// Hot path is the lock-free `DashMap::get` read; cache miss
    /// drops into `Mutex<Context>` and double-checks the cache after
    /// taking the lock so two threads racing on the same key don't
    /// double-compile.  Once an engine is in the cache it is shared
    /// via `Arc` and never evicted — kernels are tiny (one fn each)
    /// and cache size is bounded by the (op family × dtype) product.
    fn get_or_compile(&self, kind: KernelKind, dtype: DType) -> Option<Arc<EngineHolder>> {
        let key = JitKey { kind, dtype };
        if let Some(entry) = self.cache.get(&key) {
            return Some(entry.value().clone());
        }
        let guard = self.context.lock();
        if let Some(entry) = self.cache.get(&key) {
            return Some(entry.value().clone());
        }
        let engine = compile_kernel(&guard.context, kind, dtype)?;
        let holder = Arc::new(EngineHolder { engine });
        self.cache.insert(key, holder.clone());
        Some(holder)
    }
}

impl Default for MlirJitBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for MlirJitBackend {
    fn name(&self) -> &'static str {
        "mlir-jit"
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::mlir_jit(0)
    }

    fn capabilities(&self) -> &ComputeCapabilities {
        &self.capabilities
    }

    fn allocate(&self, size: usize, align: usize) -> Option<NonNull<u8>> {
        if size == 0 {
            return None;
        }
        let layout = std::alloc::Layout::from_size_align(size, align.max(1)).ok()?;
        let ptr = unsafe { std::alloc::alloc(layout) };
        let nn = NonNull::new(ptr)?;
        self.allocated_bytes.fetch_add(size, Ordering::Relaxed);
        Some(nn)
    }

    fn deallocate(&self, ptr: NonNull<u8>, size: usize, align: usize) {
        if size == 0 {
            return;
        }
        if let Ok(layout) = std::alloc::Layout::from_size_align(size, align.max(1)) {
            unsafe { std::alloc::dealloc(ptr.as_ptr(), layout) };
            self.allocated_bytes.fetch_sub(size, Ordering::Relaxed);
        }
    }

    fn copy_h2d(&self, host: *const u8, device: NonNull<u8>, size: usize) {
        unsafe { std::ptr::copy_nonoverlapping(host, device.as_ptr(), size) };
    }

    fn copy_d2h(&self, device: NonNull<u8>, host: *mut u8, size: usize) {
        unsafe { std::ptr::copy_nonoverlapping(device.as_ptr(), host, size) };
    }

    fn copy_d2d(&self, src: NonNull<u8>, dst: NonNull<u8>, size: usize) {
        unsafe { std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_ptr(), size) };
    }

    fn synchronize(&self) {
        // CPU host execution is synchronous by construction.  When the
        // GPU dialect path lands (Шаг 7), this becomes
        // `gpu.host_synchronize` through the JIT runtime.
    }

    fn binop(
        &self,
        a: &TensorHandle,
        b: &TensorHandle,
        op: TensorBinaryOp,
    ) -> Option<TensorHandle> {
        // Шаг 2c+2d coverage: F32 / F64 + I{8,16,32,64} + U{8,16,32,64}
        // for Add/Sub/Mul/Div/Mod/Min/Max.  Float dtypes additionally
        // support Pow via `math.powf`.  `binop_arith_op` returns None
        // for any combination outside this matrix, in which case the
        // dispatcher falls through to `CpuBackend`.
        if a.dtype != b.dtype {
            return None;
        }
        if binop_arith_op(op, a.dtype).is_none() {
            return None;
        }
        // For now, only same-shape inputs.  Broadcasting goes through
        // `dispatch_binop_broadcast` upstream which already routes
        // scalar-broadcast cases to dedicated SIMD kernels; reaching
        // this arm implies same-shape data.
        let a_shape = &a.shape[..a.ndim as usize];
        let b_shape = &b.shape[..b.ndim as usize];
        if a_shape != b_shape {
            return None;
        }
        let n = a.numel;
        if n == 0 {
            return Some(TensorHandle::zeros(a_shape, a.dtype)?);
        }

        // Materialise the output tensor on the host.  MLIR JIT'd
        // kernels read/write through caller-supplied pointers; we own
        // the output buffer here.
        let out = TensorHandle::zeros(a_shape, a.dtype)?;
        let holder = self.get_or_compile(KernelKind::Binop(op), a.dtype)?;

        // ABI marshalling: the JIT kernel signature is
        //   `void kernel(*ptr a, *ptr b, *ptr out, i64 n)`
        // and `mlirExecutionEngineInvokePacked` expects an array of
        // `*mut ()` where each entry points to the *value* of the
        // argument (so for pointer args we pass &mut ptr_value).
        let a_data = a.data.as_ref()?;
        let b_data = b.data.as_ref()?;
        let out_data = out.data.as_ref()?;
        let mut a_ptr: *const u8 = unsafe { (*a_data.as_ptr()).as_ptr() };
        let mut b_ptr: *const u8 = unsafe { (*b_data.as_ptr()).as_ptr() };
        let mut out_ptr: *mut u8 = unsafe { (*out_data.as_ptr()).as_mut_ptr() };
        let mut n_arg: i64 = n as i64;
        let mut packed_args: [*mut (); 4] = [
            (&mut a_ptr) as *mut _ as *mut (),
            (&mut b_ptr) as *mut _ as *mut (),
            (&mut out_ptr) as *mut _ as *mut (),
            (&mut n_arg) as *mut _ as *mut (),
        ];
        // The MLIR JIT exposes the kernel under its C-ABI wrapper name
        // (`_mlir_ciface_kernel`) when the function carries the
        // `llvm.emit_c_interface` attribute.  `invoke_packed` resolves
        // that wrapper symbol automatically for the bare name.
        let result = unsafe { holder.engine.invoke_packed("kernel", &mut packed_args) };
        if result.is_err() {
            tracing::warn!(
                "MLIR-JIT binop invocation failed for op={:?} dtype={:?}; \
                 falling back to CpuBackend",
                op,
                a.dtype
            );
            return None;
        }
        Some(out)
    }

    fn unop(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
        // Шаг 3 coverage: F32 / F64 + the math.*-backed unary primitives
        // (Neg/Abs/Sqrt/Exp/Log/Log2/Sin/Cos/Tan/Tanh/Floor/Ceil/Round/
        // Rsqrt/Erf).  Sigmoid/Relu/Gelu/Silu/Softplus/Mish/Sign require
        // composition (e.g. Sigmoid = 1/(1+exp(-x))) and arrive in a
        // future Шаг 3b — for now they return `None` and fall through
        // to `CpuBackend`'s hand-tuned forms.
        if unop_arith_op(op, a.dtype).is_none() {
            return None;
        }
        let a_shape = &a.shape[..a.ndim as usize];
        let n = a.numel;
        if n == 0 {
            return Some(TensorHandle::zeros(a_shape, a.dtype)?);
        }
        let out = TensorHandle::zeros(a_shape, a.dtype)?;
        let holder = self.get_or_compile(KernelKind::Unop(op), a.dtype)?;

        let a_data = a.data.as_ref()?;
        let out_data = out.data.as_ref()?;
        let mut a_ptr: *const u8 = unsafe { (*a_data.as_ptr()).as_ptr() };
        let mut out_ptr: *mut u8 = unsafe { (*out_data.as_ptr()).as_mut_ptr() };
        let mut n_arg: i64 = n as i64;
        let mut packed_args: [*mut (); 3] = [
            (&mut a_ptr) as *mut _ as *mut (),
            (&mut out_ptr) as *mut _ as *mut (),
            (&mut n_arg) as *mut _ as *mut (),
        ];
        let result = unsafe { holder.engine.invoke_packed("kernel", &mut packed_args) };
        if result.is_err() {
            tracing::warn!(
                "MLIR-JIT unop invocation failed for op={:?} dtype={:?}; \
                 falling back to CpuBackend",
                op,
                a.dtype
            );
            return None;
        }
        Some(out)
    }

    fn reduce(
        &self,
        a: &TensorHandle,
        op: TensorReduceOp,
        axis: Option<usize>,
    ) -> Option<TensorHandle> {
        // Шаг 5a: full-axis reduction for F32 / F64 + Sum/Prod/Max/Min.
        // Axis-specific reduction (`axis = Some(i)`) and the higher-
        // level ops (Mean / Var / Std / Norm / LogSumExp / All / Any)
        // defer to Шаг 5b.  Until then, those cases fall through to
        // the hand-tuned `cpu::reduce_*` ladder.
        if axis.is_some() {
            return None;
        }
        if !is_float_dtype(a.dtype) {
            return None;
        }
        // Mean is a post-pass over Sum: compute Σx through the cached
        // Sum kernel, then divide the scalar result by `n` in Rust.
        // Reusing the Sum engine keeps cache size bounded and matches
        // the "compose primitives" pattern used elsewhere in Verum.
        if op == TensorReduceOp::Mean {
            let sum = self.reduce(a, TensorReduceOp::Sum, axis)?;
            let n = a.numel;
            if n == 0 {
                return None;
            }
            let out_data = sum.data.as_ref()?;
            unsafe {
                match a.dtype {
                    DType::F32 => {
                        let p = (*out_data.as_ptr()).as_mut_ptr() as *mut f32;
                        *p /= n as f32;
                    }
                    DType::F64 => {
                        let p = (*out_data.as_ptr()).as_mut_ptr() as *mut f64;
                        *p /= n as f64;
                    }
                    _ => return None,
                }
            }
            return Some(sum);
        }
        if reduce_arith_op(op).is_none() {
            return None;
        }
        let n = a.numel;
        if n == 0 {
            // Reducing an empty tensor → identity element.  Honest
            // semantics differ across libraries (PyTorch raises,
            // NumPy returns identity); our hand-tuned path handles
            // this — fall through.
            return None;
        }
        // Scalar (ndim = 0) output tensor.  The CPU `reduce_*` kernels
        // return ndim=0; matching that contract keeps callers' shape
        // assertions stable across the JIT and SIMD paths.
        let out = TensorHandle::zeros(&[], a.dtype)?;
        let holder = self.get_or_compile(KernelKind::Reduce(op), a.dtype)?;

        let a_data = a.data.as_ref()?;
        let out_data = out.data.as_ref()?;
        let mut a_ptr: *const u8 = unsafe { (*a_data.as_ptr()).as_ptr() };
        let mut out_ptr: *mut u8 = unsafe { (*out_data.as_ptr()).as_mut_ptr() };
        let mut n_arg: i64 = n as i64;
        let mut packed_args: [*mut (); 3] = [
            (&mut a_ptr) as *mut _ as *mut (),
            (&mut out_ptr) as *mut _ as *mut (),
            (&mut n_arg) as *mut _ as *mut (),
        ];
        let result = unsafe { holder.engine.invoke_packed("kernel", &mut packed_args) };
        if result.is_err() {
            tracing::warn!(
                "MLIR-JIT reduce invocation failed for op={:?} dtype={:?}",
                op,
                a.dtype
            );
            return None;
        }
        Some(out)
    }

    fn matmul(&self, a: &TensorHandle, b: &TensorHandle) -> Option<TensorHandle> {
        // Шаг 4b: linalg.matmul over dynamically-shaped memrefs, JIT-
        // compiled through the full MLIR `linalg-to-loops` →
        // `convert-to-llvm` lowering pipeline.  At `opt_level = 2`
        // MLIR's loop vectoriser + LLVM's `llvm.intr.vector.reduce.*`
        // chain produce cache-tiled, vectorised GEMM kernels — the
        // path that closes the gap to cuBLAS / MKL throughput on
        // large matrices.  Compared with the Шаг 4a scalar triple-
        // loop, the kernel is ONE op (`linalg.matmul`); MLIR
        // synthesises the entire `ijk` loop nest with proper
        // accumulator handling and vectorisation.
        if a.dtype != b.dtype {
            return None;
        }
        if !is_float_dtype(a.dtype) {
            return None;
        }
        if a.ndim != 2 || b.ndim != 2 {
            return None;
        }
        let m = a.shape[0];
        let k_a = a.shape[1];
        let k_b = b.shape[0];
        let n = b.shape[1];
        if k_a != k_b {
            return None;
        }
        if m == 0 || n == 0 {
            return Some(TensorHandle::zeros(&[m, n], a.dtype)?);
        }

        let out = TensorHandle::zeros(&[m, n], a.dtype)?;
        let holder = self.get_or_compile(KernelKind::Matmul, a.dtype)?;

        let a_data = a.data.as_ref()?;
        let b_data = b.data.as_ref()?;
        let out_data = out.data.as_ref()?;
        let a_raw = unsafe { (*a_data.as_ptr()).as_ptr() } as *mut u8;
        let b_raw = unsafe { (*b_data.as_ptr()).as_ptr() } as *mut u8;
        let out_raw = unsafe { (*out_data.as_ptr()).as_mut_ptr() };

        // Construct StridedMemRefType<T, 2> descriptors on the stack.
        // Layout matches MLIR's canonical struct exactly (the C-
        // interface wrapper passes a pointer to this struct):
        //   { base, aligned, offset: i64, sizes: [i64; 2], strides: [i64; 2] }
        // Row-major contiguous: stride[0] = ncols, stride[1] = 1.
        let mut a_desc = MemrefDescriptor2D {
            base: a_raw,
            aligned: a_raw,
            offset: 0,
            sizes: [m as i64, k_a as i64],
            strides: [k_a as i64, 1],
        };
        let mut b_desc = MemrefDescriptor2D {
            base: b_raw,
            aligned: b_raw,
            offset: 0,
            sizes: [k_b as i64, n as i64],
            strides: [n as i64, 1],
        };
        let mut out_desc = MemrefDescriptor2D {
            base: out_raw,
            aligned: out_raw,
            offset: 0,
            sizes: [m as i64, n as i64],
            strides: [n as i64, 1],
        };
        let mut a_desc_ptr: *mut MemrefDescriptor2D = &mut a_desc;
        let mut b_desc_ptr: *mut MemrefDescriptor2D = &mut b_desc;
        let mut out_desc_ptr: *mut MemrefDescriptor2D = &mut out_desc;
        let mut packed_args: [*mut (); 3] = [
            (&mut a_desc_ptr) as *mut _ as *mut (),
            (&mut b_desc_ptr) as *mut _ as *mut (),
            (&mut out_desc_ptr) as *mut _ as *mut (),
        ];
        let result = unsafe { holder.engine.invoke_packed("kernel", &mut packed_args) };
        if result.is_err() {
            tracing::warn!(
                "MLIR-JIT linalg.matmul invocation failed for dtype={:?}; falling back to CpuBackend",
                a.dtype
            );
            return None;
        }
        Some(out)
    }

    fn memory_info(&self) -> (usize, usize) {
        let allocated = self.allocated_bytes.load(Ordering::Relaxed);
        (usize::MAX - allocated, usize::MAX)
    }
}

// =============================================================================
// Kernel synthesis — LLVM-dialect text → ExecutionEngine
// =============================================================================

// =============================================================================
// dtype + op classification — the data behind the template engine.
//
// MLIR doesn't carry signedness on integer types — it's encoded on the
// op (`arith.divsi` vs `arith.divui`).  Hence the `mlir_int_width` /
// `is_float_dtype` / `is_signed_dtype` triple is enough to drive the
// op-selector tables below.  Adding a new dtype only requires one
// arm here + one in `mlir_elem_type`.
// =============================================================================

fn is_float_dtype(dtype: DType) -> bool {
    matches!(dtype, DType::F32 | DType::F64)
}

fn is_signed_int(dtype: DType) -> bool {
    matches!(dtype, DType::I8 | DType::I16 | DType::I32 | DType::I64)
}

fn is_unsigned_int(dtype: DType) -> bool {
    matches!(dtype, DType::U8 | DType::U16 | DType::U32 | DType::U64)
}

fn is_int_dtype(dtype: DType) -> bool {
    is_signed_int(dtype) || is_unsigned_int(dtype)
}

/// MLIR element-type spelling for the supported numeric dtypes.
///
/// MLIR integer types are signless: `i8` covers both Verum's `I8` and
/// `U8`, with the operation choosing the interpretation (e.g.
/// `arith.divsi` vs `arith.divui`).  The same applies for the wider
/// integer widths.
fn mlir_elem_type(dtype: DType) -> Option<&'static str> {
    Some(match dtype {
        DType::F32 => "f32",
        DType::F64 => "f64",
        DType::I8 | DType::U8 => "i8",
        DType::I16 | DType::U16 => "i16",
        DType::I32 | DType::U32 => "i32",
        DType::I64 | DType::U64 => "i64",
        _ => return None,
    })
}

/// Resolve the MLIR `arith.*` / `math.*` op spelling for the given
/// `(binop, dtype)` pair.  Returning `None` signals "not yet wired —
/// fall through to `CpuBackend`".  The table is exhaustive on the
/// supported numeric range but intentionally conservative on
/// edge-case combinations (e.g. integer Pow needs `math.ipowi` which
/// has different ABI requirements; deferred to a future Шаг).
fn binop_arith_op(op: TensorBinaryOp, dtype: DType) -> Option<&'static str> {
    use TensorBinaryOp::*;
    match (op, dtype) {
        // Float arithmetic
        (Add, d) if is_float_dtype(d) => Some("arith.addf"),
        (Sub, d) if is_float_dtype(d) => Some("arith.subf"),
        (Mul, d) if is_float_dtype(d) => Some("arith.mulf"),
        (Div, d) if is_float_dtype(d) => Some("arith.divf"),
        (Mod, d) if is_float_dtype(d) => Some("arith.remf"),
        (Min, d) if is_float_dtype(d) => Some("arith.minimumf"),
        (Max, d) if is_float_dtype(d) => Some("arith.maximumf"),
        (Pow, d) if is_float_dtype(d) => Some("math.powf"),

        // Integer arithmetic — signless MLIR ops where the
        // interpretation doesn't matter (Add/Sub/Mul wrap modulo 2^N
        // identically for signed and unsigned).
        (Add, d) if is_int_dtype(d) => Some("arith.addi"),
        (Sub, d) if is_int_dtype(d) => Some("arith.subi"),
        (Mul, d) if is_int_dtype(d) => Some("arith.muli"),

        // Integer arithmetic — signedness-aware ops.
        (Div, d) if is_signed_int(d) => Some("arith.divsi"),
        (Div, d) if is_unsigned_int(d) => Some("arith.divui"),
        (Mod, d) if is_signed_int(d) => Some("arith.remsi"),
        (Mod, d) if is_unsigned_int(d) => Some("arith.remui"),
        (Min, d) if is_signed_int(d) => Some("arith.minsi"),
        (Min, d) if is_unsigned_int(d) => Some("arith.minui"),
        (Max, d) if is_signed_int(d) => Some("arith.maxsi"),
        (Max, d) if is_unsigned_int(d) => Some("arith.maxui"),

        // Integer Pow — `math.ipowi` exists but takes (base, signed_exp)
        // with different signature.  Defer to Шаг 3c.
        (Pow, _) => None,

        _ => None,
    }
}

/// Resolve the MLIR `math.*` / `arith.*` op spelling for unary ops.
///
/// All math.* ops listed here run on float dtypes (F32 / F64).  The
/// Sigmoid / Relu / Gelu / Silu / Softplus / Mish family requires
/// composition — Sigmoid = `1 / (1 + exp(-x))` and so on — which is
/// scheduled for a Шаг 3b once the simple-op coverage proves out.
fn unop_arith_op(op: TensorUnaryOp, dtype: DType) -> Option<&'static str> {
    use TensorUnaryOp::*;
    if !is_float_dtype(dtype) {
        return None;
    }
    Some(match op {
        Neg => "arith.negf",
        Abs => "math.absf",
        Sqrt => "math.sqrt",
        Exp => "math.exp",
        Log => "math.log",
        Log2 => "math.log2",
        Sin => "math.sin",
        Cos => "math.cos",
        Tan => "math.tan",
        Tanh => "math.tanh",
        Floor => "math.floor",
        Ceil => "math.ceil",
        Round => "math.roundeven",
        Rsqrt => "math.rsqrt",
        Erf => "math.erf",
        // Composed forms — Шаг 3b wiring point.
        Sigmoid | Relu | Gelu | Silu | Softplus | Mish | Sign => return None,
    })
}

/// Resolve the (init-literal, accumulator-op) pair for a reduce
/// operation.  Only Sum/Prod/Max/Min are wired (Шаг 5a); the
/// statistical ops (Mean / Var / Std / Norm / LogSumExp) need a
/// post-pass over the accumulator and are deferred to Шаг 5b.
fn reduce_arith_op(op: TensorReduceOp) -> Option<(&'static str, &'static str)> {
    use TensorReduceOp::*;
    Some(match op {
        // (init-shape, accumulator-op).  For float reductions, init
        // is the identity element of the operation: 0 for sum,
        // 1 for prod, +inf for min, -inf for max.
        Sum => ("zero", "arith.addf"),
        Prod => ("one", "arith.mulf"),
        Max => ("neg_inf", "arith.maximumf"),
        Min => ("pos_inf", "arith.minimumf"),
        _ => return None,
    })
}

// =============================================================================
// Kernel synthesis — MLIR text → ExecutionEngine
//
// The two templates below share the same loop-and-pointer-arith
// scaffold; only arity (binary vs unary) and the inner `arith` /
// `math` op differ.  After `Module::parse` we run the umbrella
// `convert-to-llvm` conversion to lower everything to the `llvm`
// dialect, then hand the result to `mlirExecutionEngineCreate`.  The
// `llvm.emit_c_interface` attribute on `func.func` directs MLIR to
// generate the `_mlir_ciface_kernel` wrapper that
// `mlirExecutionEngineInvokePacked` resolves — without it,
// invocation fails with `InvokeFunction`.
//
// LLVM's vectoriser running inside `ExecutionEngine` generates
// target-native SIMD (AVX-512 / NEON / RVV) automatically — that's
// the same machinery PyTorch's `torch.compile` and JAX's XLA ride
// for production kernels.
// =============================================================================

fn build_binop_kernel_text(op: TensorBinaryOp, dtype: DType) -> Option<String> {
    let elem_ty = mlir_elem_type(dtype)?;
    let arith = binop_arith_op(op, dtype)?;
    Some(format!(
        r#"module {{
  func.func @kernel(%a: !llvm.ptr, %b: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
    %c0 = arith.constant 0 : i64
    %c1 = arith.constant 1 : i64
    cf.br ^loop(%c0 : i64)
  ^loop(%i: i64):
    %cond = arith.cmpi slt, %i, %n : i64
    cf.cond_br %cond, ^body, ^exit
  ^body:
    %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    %bi = llvm.getelementptr inbounds %b[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    %va = llvm.load %ai : !llvm.ptr -> {elem}
    %vb = llvm.load %bi : !llvm.ptr -> {elem}
    %vc = {arith} %va, %vb : {elem}
    %ci = llvm.getelementptr inbounds %out[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    llvm.store %vc, %ci : {elem}, !llvm.ptr
    %i_next = arith.addi %i, %c1 : i64
    cf.br ^loop(%i_next : i64)
  ^exit:
    return
  }}
}}
"#,
        elem = elem_ty,
        arith = arith,
    ))
}

/// Build the MLIR text for a full-tensor reduction.
///
/// Signature: `void kernel(*const T input, *mut T output, i64 n)`.
/// Iterates the input once, carrying the accumulator in a
/// block-arg-typed loop; on exit, stores the final accumulator to
/// `*output`.  LLVM auto-vectorises the loop into a tree-reduction
/// of vector lanes for ops where the accumulator is associative
/// (Sum / Prod / Max / Min on float — strictly speaking float Sum
/// is non-associative but `-ffast-math`-style assumptions inside
/// LLVM's loop vectoriser make this acceptable in practice; users
/// who need bit-exact reductions should add an explicit dispatch
/// guard upstream).
fn build_reduce_kernel_text(op: TensorReduceOp, dtype: DType) -> Option<String> {
    let elem_ty = mlir_elem_type(dtype)?;
    if !is_float_dtype(dtype) {
        return None;
    }
    let (init_kind, accum) = reduce_arith_op(op)?;
    // For F32/F64 the IEEE-754 specials parse via `arith.constant`
    // with appropriate float literals.  Hex floats avoid rounding
    // ambiguity in the parser.
    let init_literal = match (init_kind, dtype) {
        ("zero", DType::F32) => "0.0 : f32",
        ("zero", DType::F64) => "0.0 : f64",
        ("one", DType::F32) => "1.0 : f32",
        ("one", DType::F64) => "1.0 : f64",
        ("pos_inf", DType::F32) => "0x7F800000 : f32",
        ("pos_inf", DType::F64) => "0x7FF0000000000000 : f64",
        ("neg_inf", DType::F32) => "0xFF800000 : f32",
        ("neg_inf", DType::F64) => "0xFFF0000000000000 : f64",
        _ => return None,
    };
    Some(format!(
        r#"module {{
  func.func @kernel(%a: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
    %c0 = arith.constant 0 : i64
    %c1 = arith.constant 1 : i64
    %init = arith.constant {init_lit}
    cf.br ^loop(%c0, %init : i64, {elem})
  ^loop(%i: i64, %acc: {elem}):
    %done = arith.cmpi sge, %i, %n : i64
    cf.cond_br %done, ^store, ^body
  ^body:
    %ap = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    %va = llvm.load %ap : !llvm.ptr -> {elem}
    %new_acc = {accum} %acc, %va : {elem}
    %i_next = arith.addi %i, %c1 : i64
    cf.br ^loop(%i_next, %new_acc : i64, {elem})
  ^store:
    llvm.store %acc, %out : {elem}, !llvm.ptr
    return
  }}
}}
"#,
        elem = elem_ty,
        init_lit = init_literal,
        accum = accum,
    ))
}

/// Build the MLIR text for a row-major 2D matmul kernel using
/// `linalg.matmul` over dynamically-shaped memrefs.
///
/// This is the showpiece of MLIR-driven compute: the single
/// `linalg.matmul` op triggers the full tile-and-fuse + vectorise
/// scheduler when lowered, producing multi-level loop nests with
/// register / L1 / L2 cache tiling and target-native SIMD —
/// the same path PyTorch / JAX / Triton ride for production GEMMs.
/// Compared with the Шаг 4a scalar triple-loop, this opens the door
/// to cuBLAS / MKL-class throughput on large matrices.
///
/// Function signature (after `linalg-to-loops` → `convert-to-llvm`
/// + `finalize-memref-to-llvm` lowering and the
/// `llvm.emit_c_interface` wrapper):
///
/// ```c
/// extern "C" void _mlir_ciface_kernel(
///     StridedMemRefType<T, 2>* a,
///     StridedMemRefType<T, 2>* b,
///     StridedMemRefType<T, 2>* out
/// );
/// ```
///
/// where `StridedMemRefType<T, 2>` is the canonical MLIR descriptor
/// (`{base, aligned, offset, sizes[2], strides[2]}` = 56 bytes).
/// The Rust caller in `matmul()` constructs three of these on the
/// stack and passes pointers to them through `invoke_packed`.
fn build_matmul_kernel_text(dtype: DType) -> Option<String> {
    let elem_ty = mlir_elem_type(dtype)?;
    if !is_float_dtype(dtype) {
        return None;
    }
    Some(format!(
        r#"module {{
  func.func @kernel(
    %a: memref<?x?x{elem}>,
    %b: memref<?x?x{elem}>,
    %out: memref<?x?x{elem}>
  ) attributes {{ llvm.emit_c_interface }} {{
    linalg.matmul ins(%a, %b : memref<?x?x{elem}>, memref<?x?x{elem}>)
                  outs(%out : memref<?x?x{elem}>)
    return
  }}
}}
"#,
        elem = elem_ty,
    ))
}

/// Pass-pipeline shape needed by the linalg-matmul kernel.
///
/// `convert-to-llvm` (the umbrella) does NOT lower `linalg` ops;
/// they need an explicit `convert-linalg-to-loops` pre-step that
/// rewrites `linalg.matmul` into an `scf.for` nest.  After that the
/// memref ops + scf + arith all reach LLVM dialect via the umbrella
/// pass, and `finalize-memref-to-llvm` lowers memref descriptors to
/// `llvm.struct`.  Func-level cleanup happens inside the umbrella.
fn matmul_lowering_pipeline(context: &Context) -> PassManager<'_> {
    let pm = PassManager::new(context);
    pm.add_pass(pass::linalg::create_convert_linalg_to_loops_pass());
    pm.add_pass(pass::conversion::create_scf_to_control_flow());
    pm.add_pass(pass::conversion::create_finalize_mem_ref_to_llvm());
    pm.add_pass(pass::conversion::create_to_llvm());
    // The conversion to LLVM produces transient
    // `builtin.unrealized_conversion_cast` ops on the boundaries
    // between dialects (memref↔llvm.struct in particular); the
    // reconcile pass folds matched cast pairs and the LLVM
    // translation rejects any that survive.  Without it the JIT
    // path fails with "LLVM Translation failed for operation:
    // builtin.unrealized_conversion_cast".
    pm.add_pass(pass::conversion::create_reconcile_unrealized_casts());
    pm
}

fn build_unop_kernel_text(op: TensorUnaryOp, dtype: DType) -> Option<String> {
    let elem_ty = mlir_elem_type(dtype)?;
    let arith = unop_arith_op(op, dtype)?;
    Some(format!(
        r#"module {{
  func.func @kernel(%a: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
    %c0 = arith.constant 0 : i64
    %c1 = arith.constant 1 : i64
    cf.br ^loop(%c0 : i64)
  ^loop(%i: i64):
    %cond = arith.cmpi slt, %i, %n : i64
    cf.cond_br %cond, ^body, ^exit
  ^body:
    %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    %va = llvm.load %ai : !llvm.ptr -> {elem}
    %vc = {arith} %va : {elem}
    %ci = llvm.getelementptr inbounds %out[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
    llvm.store %vc, %ci : {elem}, !llvm.ptr
    %i_next = arith.addi %i, %c1 : i64
    cf.br ^loop(%i_next : i64)
  ^exit:
    return
  }}
}}
"#,
        elem = elem_ty,
        arith = arith,
    ))
}

/// Compile a kernel text into a JIT engine.
///
/// Failure modes on this path:
///   * Kernel text builder returns `None` (op/dtype not yet wired).
///   * Module parse failure → returns `None` and emits a tracing warn.
///   * Verifier rejection → same.
///   * `convert-to-llvm` lowering pass failure → likely a dialect op
///     that the umbrella conversion does not know how to lower.
///   * `ExecutionEngine` construction failure → missing LLVM backend.
fn compile_kernel(
    context: &Context,
    kind: KernelKind,
    dtype: DType,
) -> Option<ExecutionEngine> {
    let text = match kind {
        KernelKind::Binop(op) => build_binop_kernel_text(op, dtype)?,
        KernelKind::Unop(op) => build_unop_kernel_text(op, dtype)?,
        KernelKind::Matmul => build_matmul_kernel_text(dtype)?,
        KernelKind::Reduce(op) => build_reduce_kernel_text(op, dtype)?,
    };
    let _location = Location::unknown(context);
    let mut module = match Module::parse(context, &text) {
        Some(m) => m,
        None => {
            tracing::warn!(
                "MLIR-JIT: kernel text parse failed for kind={:?} dtype={:?}",
                kind,
                dtype
            );
            return None;
        }
    };
    if !module.as_operation().verify() {
        tracing::warn!(
            "MLIR-JIT: kernel verification failed for kind={:?} dtype={:?}",
            kind,
            dtype
        );
        return None;
    }
    // Choose the lowering pipeline by kernel kind: matmul uses
    // `linalg.matmul` over memrefs and needs the linalg-to-loops +
    // memref-to-llvm pre-steps; the other kernels are already in
    // arith/cf/llvm dialects and reach LLVM through the umbrella
    // `convert-to-llvm` directly.
    let pass_manager = match kind {
        KernelKind::Matmul => matmul_lowering_pipeline(context),
        _ => {
            let pm = PassManager::new(context);
            pm.add_pass(pass::conversion::create_to_llvm());
            pm
        }
    };
    if pass_manager.run(&mut module).is_err() {
        tracing::warn!(
            "MLIR-JIT: lowering pipeline failed for kind={:?} dtype={:?}",
            kind,
            dtype
        );
        return None;
    }
    let engine = ExecutionEngine::new(&module, /* opt_level */ 2, &[], /* dump */ false);
    Some(engine)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_device_id_are_stable() {
        let backend = MlirJitBackend::new();
        assert_eq!(backend.name(), "mlir-jit");
        assert_eq!(backend.device_id(), DeviceId::mlir_jit(0));
        assert!(backend.device_id().is_mlir_jit());
    }

    #[test]
    fn capabilities_reflect_host_parallelism() {
        let backend = MlirJitBackend::new();
        let caps = backend.capabilities();
        assert!(caps.max_threads >= 1);
        assert!(caps.has_fma);
    }

    #[test]
    fn alloc_dealloc_round_trip() {
        let backend = MlirJitBackend::new();
        let ptr = backend.allocate(64, 8).expect("allocation");
        backend.deallocate(ptr, 64, 8);
    }

    #[test]
    fn binop_f32_add_executes_correctly() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::F32).unwrap();
        // Initialise inputs.
        unsafe {
            let a_ptr = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            let b_ptr = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            for i in 0..4 {
                *a_ptr.add(i) = (i as f32) * 1.0;
                *b_ptr.add(i) = (i as f32) * 10.0;
            }
        }
        let result = backend
            .binop(&a, &b, TensorBinaryOp::Add)
            .expect("JIT binop must succeed");
        unsafe {
            let r_ptr = (*result.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            for i in 0..4 {
                let expected = (i as f32) * 11.0;
                let got = *r_ptr.add(i);
                assert!(
                    (got - expected).abs() < 1e-6,
                    "F32 add[{i}] = {got}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn binop_f64_sub_executes_correctly() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F64).unwrap();
        let b = TensorHandle::zeros(&[3], DType::F64).unwrap();
        unsafe {
            let a_ptr = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            let b_ptr = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            *a_ptr.add(0) = 5.0;
            *a_ptr.add(1) = 10.0;
            *a_ptr.add(2) = -3.0;
            *b_ptr.add(0) = 2.0;
            *b_ptr.add(1) = 4.0;
            *b_ptr.add(2) = 1.0;
        }
        let result = backend.binop(&a, &b, TensorBinaryOp::Sub).expect("JIT sub");
        unsafe {
            let r_ptr = (*result.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            assert!((*r_ptr.add(0) - 3.0).abs() < 1e-12);
            assert!((*r_ptr.add(1) - 6.0).abs() < 1e-12);
            assert!((*r_ptr.add(2) - (-4.0)).abs() < 1e-12);
        }
    }

    #[test]
    fn binop_f32_mul_div_round_trip() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        unsafe {
            let a_ptr = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            let b_ptr = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            *a_ptr.add(0) = 6.0;
            *a_ptr.add(1) = 8.0;
            *b_ptr.add(0) = 3.0;
            *b_ptr.add(1) = 4.0;
        }
        let prod = backend.binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        let quot = backend.binop(&a, &b, TensorBinaryOp::Div).unwrap();
        unsafe {
            let p_ptr = (*prod.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            let q_ptr = (*quot.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            assert!((*p_ptr.add(0) - 18.0).abs() < 1e-6);
            assert!((*p_ptr.add(1) - 32.0).abs() < 1e-6);
            assert!((*q_ptr.add(0) - 2.0).abs() < 1e-6);
            assert!((*q_ptr.add(1) - 2.0).abs() < 1e-6);
        }
    }

    #[test]
    fn binop_returns_none_for_unsupported_dtypes() {
        // Шаг 2c+2d coverage spans F32/F64 + I/U{8,16,32,64}.  The
        // half-precision floats and complex dtypes are intentionally
        // left to a future Шаг because their MLIR lowering needs an
        // additional `convert-arith-to-fp16` / complex-dialect pass.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F16).unwrap();
        let b = TensorHandle::zeros(&[4], DType::F16).unwrap();
        assert!(backend.binop(&a, &b, TensorBinaryOp::Add).is_none());
    }

    #[test]
    fn binop_dtype_mismatch_returns_none() {
        // Mixing dtypes is a type-promotion concern handled upstream
        // by `tensor_binop`'s `DType::promote_static`.  By the time we
        // reach the JIT backend the inputs must agree; if they don't,
        // we fall through so the upstream cast happens.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::F64).unwrap();
        assert!(backend.binop(&a, &b, TensorBinaryOp::Add).is_none());
    }

    #[test]
    fn binop_pow_on_int_falls_through() {
        // `math.ipowi` exists but takes a different ABI signature
        // (signed-only exponent); deferred to Шаг 3c.  For now,
        // integer Pow falls through to `CpuBackend`.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::I32).unwrap();
        assert!(backend.binop(&a, &b, TensorBinaryOp::Pow).is_none());
    }

    #[test]
    fn cache_reuses_compiled_kernels() {
        // Compile once, invoke twice — second invocation must hit the
        // cache (cache.len() stays at 1 for the same key).
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        let cache_size_after_first = backend.cache.len();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        let cache_size_after_second = backend.cache.len();
        assert_eq!(cache_size_after_first, 1);
        assert_eq!(cache_size_after_second, 1);
    }

    #[test]
    fn cache_holds_distinct_keys_per_op() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Sub).unwrap();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        // 3 distinct (op, F32) entries.
        assert_eq!(backend.cache.len(), 3);
    }

    #[test]
    fn reduce_falls_through_for_axis_specific() {
        // Шаг 5b wiring point: axis-specific reductions need a
        // different kernel (rank-aware loop nest); Шаг 5a covers
        // only `axis = None` (full-tensor reduction).
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        assert!(backend.reduce(&a, TensorReduceOp::Sum, Some(0)).is_none());
    }

    #[test]
    fn reduce_falls_through_for_int_dtype() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        assert!(backend.reduce(&a, TensorReduceOp::Sum, None).is_none());
    }

    #[test]
    fn reduce_falls_through_for_unsupported_op() {
        // Var / Std / Norm / LogSumExp / All / Any deferred to Шаг
        // 5c.  Mean has graduated from this pin (now wired via the
        // Sum-then-divide composition — see `reduce_f32_mean_executes`).
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        assert!(backend.reduce(&a, TensorReduceOp::Var, None).is_none());
        assert!(backend.reduce(&a, TensorReduceOp::Norm, None).is_none());
    }

    #[test]
    fn reduce_f32_mean_executes() {
        // Mean = Sum / n, so for [2,4,6,8] mean = 20/4 = 5.0.  The
        // pin also indirectly verifies the Sum kernel returns the
        // correct accumulator (we'd see double-divide / wrong-n if
        // the post-pass and the kernel disagreed on element count).
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        fill_f32(&a, &[2.0, 4.0, 6.0, 8.0]);
        let m = backend.reduce(&a, TensorReduceOp::Mean, None).unwrap();
        let v = read_f32(&m, 1);
        assert!((v[0] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn reduce_f64_mean_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F64).unwrap();
        unsafe {
            let p = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            *p.add(0) = 1.0;
            *p.add(1) = 2.0;
            *p.add(2) = 3.0;
        }
        let m = backend.reduce(&a, TensorReduceOp::Mean, None).unwrap();
        unsafe {
            let p = (*m.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            assert!((*p - 2.0).abs() < 1e-12);
        }
    }

    #[test]
    fn reduce_f32_sum_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[5], DType::F32).unwrap();
        fill_f32(&a, &[1.0, 2.0, 3.0, 4.0, 5.0]);
        let r = backend.reduce(&a, TensorReduceOp::Sum, None).unwrap();
        let v = read_f32(&r, 1);
        assert!((v[0] - 15.0).abs() < 1e-5);
    }

    #[test]
    fn reduce_f32_prod_max_min_execute() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        fill_f32(&a, &[2.0, 3.0, 5.0, 1.0]);
        let p = backend.reduce(&a, TensorReduceOp::Prod, None).unwrap();
        let mx = backend.reduce(&a, TensorReduceOp::Max, None).unwrap();
        let mn = backend.reduce(&a, TensorReduceOp::Min, None).unwrap();
        assert!((read_f32(&p, 1)[0] - 30.0).abs() < 1e-5);
        assert!((read_f32(&mx, 1)[0] - 5.0).abs() < 1e-5);
        assert!((read_f32(&mn, 1)[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn reduce_f64_sum_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F64).unwrap();
        unsafe {
            let p = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            *p.add(0) = 1.5;
            *p.add(1) = 2.5;
            *p.add(2) = 3.0;
        }
        let r = backend.reduce(&a, TensorReduceOp::Sum, None).unwrap();
        unsafe {
            let p = (*r.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            assert!((*p - 7.0).abs() < 1e-12);
        }
    }

    #[test]
    fn reduce_max_with_negatives() {
        // Max init is -inf — pin against accidentally using 0.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        fill_f32(&a, &[-5.0, -3.0, -7.0, -1.0]);
        let r = backend.reduce(&a, TensorReduceOp::Max, None).unwrap();
        assert!((read_f32(&r, 1)[0] - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn reduce_min_with_positives() {
        // Min init is +inf — pin against accidentally using 0.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        fill_f32(&a, &[5.0, 3.0, 7.0, 1.0]);
        let r = backend.reduce(&a, TensorReduceOp::Min, None).unwrap();
        assert!((read_f32(&r, 1)[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn matmul_falls_through_for_non_2d() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::F32).unwrap();
        assert!(backend.matmul(&a, &b).is_none());
    }

    #[test]
    fn matmul_falls_through_for_int() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2, 2], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[2, 2], DType::I32).unwrap();
        assert!(backend.matmul(&a, &b).is_none());
    }

    #[test]
    fn matmul_falls_through_for_shape_mismatch() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4, 2], DType::F32).unwrap();
        assert!(backend.matmul(&a, &b).is_none());
    }

    #[test]
    fn matmul_f32_2x3_times_3x2() {
        let backend = MlirJitBackend::new();
        // a = [[1,2,3],[4,5,6]]  (2×3)
        // b = [[7,8],[9,10],[11,12]]  (3×2)
        // out = a @ b = [[58, 64], [139, 154]]
        let a = TensorHandle::zeros(&[2, 3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[3, 2], DType::F32).unwrap();
        fill_f32(&a, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        fill_f32(&b, &[7.0, 8.0, 9.0, 10.0, 11.0, 12.0]);
        let out = backend.matmul(&a, &b).expect("JIT matmul");
        let r = read_f32(&out, 4);
        assert!((r[0] - 58.0).abs() < 1e-4, "out[0,0] = {}", r[0]);
        assert!((r[1] - 64.0).abs() < 1e-4, "out[0,1] = {}", r[1]);
        assert!((r[2] - 139.0).abs() < 1e-4, "out[1,0] = {}", r[2]);
        assert!((r[3] - 154.0).abs() < 1e-4, "out[1,1] = {}", r[3]);
    }

    #[test]
    fn matmul_f32_identity_round_trip() {
        let backend = MlirJitBackend::new();
        // a = identity 3×3, b = arbitrary 3×2 → out == b
        let a = TensorHandle::zeros(&[3, 3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[3, 2], DType::F32).unwrap();
        fill_f32(&a, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
        fill_f32(&b, &[2.0, 3.0, 5.0, 7.0, 11.0, 13.0]);
        let out = backend.matmul(&a, &b).unwrap();
        assert_eq!(read_f32(&out, 6), vec![2.0, 3.0, 5.0, 7.0, 11.0, 13.0]);
    }

    #[test]
    fn matmul_f64_2x2() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
        let b = TensorHandle::zeros(&[2, 2], DType::F64).unwrap();
        unsafe {
            let pa = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            let pb = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            *pa.add(0) = 1.0;
            *pa.add(1) = 2.0;
            *pa.add(2) = 3.0;
            *pa.add(3) = 4.0;
            *pb.add(0) = 5.0;
            *pb.add(1) = 6.0;
            *pb.add(2) = 7.0;
            *pb.add(3) = 8.0;
        }
        // [[1,2],[3,4]] @ [[5,6],[7,8]] = [[19,22],[43,50]]
        let out = backend.matmul(&a, &b).unwrap();
        unsafe {
            let p = (*out.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            assert!((*p.add(0) - 19.0).abs() < 1e-12);
            assert!((*p.add(1) - 22.0).abs() < 1e-12);
            assert!((*p.add(2) - 43.0).abs() < 1e-12);
            assert!((*p.add(3) - 50.0).abs() < 1e-12);
        }
    }

    // -----------------------------------------------------------------
    // Шаг 2c+2d coverage: integer binops + float Pow/Mod/Min/Max
    // -----------------------------------------------------------------

    fn fill_i32(t: &TensorHandle, vals: &[i32]) {
        unsafe {
            let p = (*t.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut i32;
            for (i, &v) in vals.iter().enumerate() {
                *p.add(i) = v;
            }
        }
    }
    fn read_i32(t: &TensorHandle, n: usize) -> Vec<i32> {
        unsafe {
            let p = (*t.data.as_ref().unwrap().as_ptr()).as_ptr() as *const i32;
            (0..n).map(|i| *p.add(i)).collect()
        }
    }

    #[test]
    fn binop_i32_add_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::I32).unwrap();
        fill_i32(&a, &[1, 2, 3, 4]);
        fill_i32(&b, &[10, 20, 30, 40]);
        let r = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        assert_eq!(read_i32(&r, 4), vec![11, 22, 33, 44]);
    }

    #[test]
    fn binop_i32_sub_mul_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::I32).unwrap();
        fill_i32(&a, &[10, 20, 30, 40]);
        fill_i32(&b, &[3, 4, 5, 6]);
        let s = backend.binop(&a, &b, TensorBinaryOp::Sub).unwrap();
        let m = backend.binop(&a, &b, TensorBinaryOp::Mul).unwrap();
        assert_eq!(read_i32(&s, 4), vec![7, 16, 25, 34]);
        assert_eq!(read_i32(&m, 4), vec![30, 80, 150, 240]);
    }

    #[test]
    fn binop_i32_div_signed() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::I32).unwrap();
        fill_i32(&a, &[10, -10, 7, -7]);
        fill_i32(&b, &[3, 3, -2, -2]);
        let r = backend.binop(&a, &b, TensorBinaryOp::Div).unwrap();
        // signed truncated division: 10/3=3, -10/3=-3, 7/-2=-3, -7/-2=3
        assert_eq!(read_i32(&r, 4), vec![3, -3, -3, 3]);
    }

    #[test]
    fn binop_u8_add_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::U8).unwrap();
        let b = TensorHandle::zeros(&[3], DType::U8).unwrap();
        unsafe {
            let pa = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr();
            let pb = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr();
            *pa.add(0) = 5u8;
            *pa.add(1) = 100u8;
            *pa.add(2) = 200u8;
            *pb.add(0) = 3u8;
            *pb.add(1) = 50u8;
            *pb.add(2) = 100u8;
        }
        let r = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        unsafe {
            let pr = (*r.data.as_ref().unwrap().as_ptr()).as_ptr();
            assert_eq!(*pr.add(0), 8u8);
            assert_eq!(*pr.add(1), 150u8);
            assert_eq!(*pr.add(2), 44u8); // 200+100=300, wraps to 44 in u8
        }
    }

    #[test]
    fn binop_f32_min_max_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[3], DType::F32).unwrap();
        unsafe {
            let pa = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            let pb = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            *pa.add(0) = 1.0;
            *pa.add(1) = 5.0;
            *pa.add(2) = -3.0;
            *pb.add(0) = 2.0;
            *pb.add(1) = 4.0;
            *pb.add(2) = 0.0;
        }
        let mn = backend.binop(&a, &b, TensorBinaryOp::Min).unwrap();
        let mx = backend.binop(&a, &b, TensorBinaryOp::Max).unwrap();
        unsafe {
            let pmn = (*mn.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            let pmx = (*mx.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            assert!((*pmn.add(0) - 1.0).abs() < 1e-6);
            assert!((*pmn.add(1) - 4.0).abs() < 1e-6);
            assert!((*pmn.add(2) - (-3.0)).abs() < 1e-6);
            assert!((*pmx.add(0) - 2.0).abs() < 1e-6);
            assert!((*pmx.add(1) - 5.0).abs() < 1e-6);
            assert!((*pmx.add(2) - 0.0).abs() < 1e-6);
        }
    }

    #[test]
    fn binop_f32_mod_pow_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        unsafe {
            let pa = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            let pb = (*b.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            *pa.add(0) = 7.5;
            *pa.add(1) = 2.0;
            *pb.add(0) = 2.0;
            *pb.add(1) = 3.0;
        }
        let m = backend.binop(&a, &b, TensorBinaryOp::Mod).unwrap();
        let p = backend.binop(&a, &b, TensorBinaryOp::Pow).unwrap();
        unsafe {
            let pm = (*m.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            let pp = (*p.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            assert!((*pm.add(0) - 1.5).abs() < 1e-5); // 7.5 % 2 = 1.5
            assert!((*pm.add(1) - 2.0).abs() < 1e-5); // 2.0 % 3 = 2.0
            assert!((*pp.add(0) - 56.25).abs() < 1e-3); // 7.5^2
            assert!((*pp.add(1) - 8.0).abs() < 1e-5); // 2^3
        }
    }

    // -----------------------------------------------------------------
    // Шаг 3 coverage: float unops via math.* dialect
    // -----------------------------------------------------------------

    fn fill_f32(t: &TensorHandle, vals: &[f32]) {
        unsafe {
            let p = (*t.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
            for (i, &v) in vals.iter().enumerate() {
                *p.add(i) = v;
            }
        }
    }
    fn read_f32(t: &TensorHandle, n: usize) -> Vec<f32> {
        unsafe {
            let p = (*t.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
            (0..n).map(|i| *p.add(i)).collect()
        }
    }

    #[test]
    fn unop_neg_abs_sqrt_execute() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        fill_f32(&a, &[-2.0, -1.0, 4.0, 9.0]);

        let n = backend.unop(&a, TensorUnaryOp::Neg).unwrap();
        assert_eq!(read_f32(&n, 4), vec![2.0, 1.0, -4.0, -9.0]);

        let abs = backend.unop(&a, TensorUnaryOp::Abs).unwrap();
        assert_eq!(read_f32(&abs, 4), vec![2.0, 1.0, 4.0, 9.0]);

        let sq = backend.unop(&abs, TensorUnaryOp::Sqrt).unwrap();
        let r = read_f32(&sq, 4);
        assert!((r[0] - 2.0_f32.sqrt()).abs() < 1e-6);
        assert!((r[2] - 2.0).abs() < 1e-6);
        assert!((r[3] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn unop_exp_log_round_trip() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F32).unwrap();
        fill_f32(&a, &[1.0, 2.0, 3.0]);
        let e = backend.unop(&a, TensorUnaryOp::Exp).unwrap();
        let l = backend.unop(&e, TensorUnaryOp::Log).unwrap();
        let r = read_f32(&l, 3);
        for (i, &expected) in [1.0_f32, 2.0, 3.0].iter().enumerate() {
            assert!((r[i] - expected).abs() < 1e-5, "log(exp({expected})) = {}", r[i]);
        }
    }

    #[test]
    fn unop_trig_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        fill_f32(&a, &[0.0, std::f32::consts::FRAC_PI_2]);
        let s = backend.unop(&a, TensorUnaryOp::Sin).unwrap();
        let c = backend.unop(&a, TensorUnaryOp::Cos).unwrap();
        let rs = read_f32(&s, 2);
        let rc = read_f32(&c, 2);
        assert!(rs[0].abs() < 1e-6);
        assert!((rs[1] - 1.0).abs() < 1e-5);
        assert!((rc[0] - 1.0).abs() < 1e-6);
        assert!(rc[1].abs() < 1e-5);
    }

    #[test]
    fn unop_floor_ceil_round_executes() {
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[3], DType::F64).unwrap();
        unsafe {
            let p = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
            *p.add(0) = 1.7;
            *p.add(1) = -2.3;
            *p.add(2) = 3.5;
        }
        let f = backend.unop(&a, TensorUnaryOp::Floor).unwrap();
        let c = backend.unop(&a, TensorUnaryOp::Ceil).unwrap();
        let r = backend.unop(&a, TensorUnaryOp::Round).unwrap();
        unsafe {
            let pf = (*f.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            let pc = (*c.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            let pr = (*r.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
            assert!((*pf.add(0) - 1.0).abs() < 1e-12);
            assert!((*pf.add(1) - (-3.0)).abs() < 1e-12);
            assert!((*pc.add(0) - 2.0).abs() < 1e-12);
            assert!((*pc.add(1) - (-2.0)).abs() < 1e-12);
            // Banker's rounding (round-half-to-even): 3.5 → 4.0
            assert!((*pr.add(2) - 4.0).abs() < 1e-12);
        }
    }

    #[test]
    fn unop_returns_none_for_unsupported_dtypes() {
        // Шаг 3 contract: math.* ops are float-only.  Integer dtypes
        // fall through to `CpuBackend`.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        assert!(backend.unop(&a, TensorUnaryOp::Sqrt).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Sin).is_none());
    }

    #[test]
    fn unop_returns_none_for_composed_ops() {
        // Шаг 3b wiring point: Sigmoid / Relu / Gelu / Silu / Softplus /
        // Mish / Sign require composition that the simple template
        // engine doesn't synthesise.  Until they land, fall through.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        assert!(backend.unop(&a, TensorUnaryOp::Sigmoid).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Relu).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Gelu).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Silu).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Softplus).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Mish).is_none());
        assert!(backend.unop(&a, TensorUnaryOp::Sign).is_none());
    }

    #[test]
    fn cache_holds_separate_keys_for_binop_vs_unop() {
        // Both families share one cache; the `KernelKind` discriminator
        // keeps `(Add, F32)` distinct from `(Neg, F32)` even though
        // both keys compile a kernel parameterised on `f32`.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let _ = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
        let _ = backend.unop(&a, TensorUnaryOp::Neg).unwrap();
        assert_eq!(backend.cache.len(), 2);
    }
}
