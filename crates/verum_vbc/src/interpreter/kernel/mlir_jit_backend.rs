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
// JIT key + cache holder
// =============================================================================

/// Cache key for compiled kernels.  A hit on this key means the
/// `(op, dtype)` pair has been lowered to LLVM dialect, JIT-compiled,
/// and the resulting function pointer is ready for `invoke_packed`.
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
struct JitKey {
    op: TensorBinaryOp,
    dtype: DType,
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

    /// Look up the cached engine for (op, dtype) or compile and insert.
    fn get_or_compile_binop(&self, op: TensorBinaryOp, dtype: DType) -> Option<Arc<EngineHolder>> {
        let key = JitKey { op, dtype };
        if let Some(entry) = self.cache.get(&key) {
            return Some(entry.value().clone());
        }
        // Cache miss — serialise compile under the Context lock so
        // two threads racing on the same key don't double-compile.
        let guard = self.context.lock();
        // Re-check the cache: another thread may have compiled while
        // we were waiting on the lock.
        if let Some(entry) = self.cache.get(&key) {
            return Some(entry.value().clone());
        }
        let engine = compile_binop_kernel(&guard.context, op, dtype)?;
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
        // Шаг 2 contract: only F32 / F64 + Add/Sub/Mul/Div are wired.
        // Other dtypes / ops return None so the dispatcher falls
        // through to `CpuBackend`.  Шаг 2c will widen to integer
        // dtypes (i8/i16/i32/i64 + their u-variants); Шаг 2d adds
        // Pow/Mod/Min/Max via `math.*` ops.
        if a.dtype != b.dtype {
            return None;
        }
        if !matches!(a.dtype, DType::F32 | DType::F64) {
            return None;
        }
        if !matches!(
            op,
            TensorBinaryOp::Add | TensorBinaryOp::Sub | TensorBinaryOp::Mul | TensorBinaryOp::Div
        ) {
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
        let holder = self.get_or_compile_binop(op, a.dtype)?;

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

    fn unop(&self, _a: &TensorHandle, _op: TensorUnaryOp) -> Option<TensorHandle> {
        // Шаг 3 wiring point.
        None
    }

    fn reduce(
        &self,
        _a: &TensorHandle,
        _op: TensorReduceOp,
        _axis: Option<usize>,
    ) -> Option<TensorHandle> {
        // Шаг 5 wiring point.
        None
    }

    fn matmul(&self, _a: &TensorHandle, _b: &TensorHandle) -> Option<TensorHandle> {
        // Шаг 4 wiring point.
        None
    }

    fn memory_info(&self) -> (usize, usize) {
        let allocated = self.allocated_bytes.load(Ordering::Relaxed);
        (usize::MAX - allocated, usize::MAX)
    }
}

// =============================================================================
// Kernel synthesis — LLVM-dialect text → ExecutionEngine
// =============================================================================

/// Build the MLIR text for an element-wise binop.
///
/// The kernel uses a mix of `func`/`arith`/`cf`/`llvm` dialects.  After
/// `Module::parse` we run the umbrella `convert-to-llvm` conversion
/// pass to lower everything to the `llvm` dialect, then hand the
/// result to `mlirExecutionEngineCreate`.  The `llvm.emit_c_interface`
/// attribute on `func.func` directs MLIR to generate the C-ABI wrapper
/// (`_mlir_ciface_kernel`) that `mlirExecutionEngineInvokePacked`
/// looks up — without it, `invoke_packed` returns `InvokeFunction`.
///
/// LLVM's vectoriser running inside `ExecutionEngine` generates
/// target-native SIMD (AVX-512 / NEON / RVV) automatically — that's
/// the same machinery PyTorch's `torch.compile` rides for its
/// auto-tuned kernels.
fn build_binop_kernel_text(op: TensorBinaryOp, dtype: DType) -> Option<String> {
    let elem_ty = match dtype {
        DType::F32 => "f32",
        DType::F64 => "f64",
        _ => return None,
    };
    let arith_op = match op {
        TensorBinaryOp::Add => "arith.addf",
        TensorBinaryOp::Sub => "arith.subf",
        TensorBinaryOp::Mul => "arith.mulf",
        TensorBinaryOp::Div => "arith.divf",
        _ => return None,
    };
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
        arith = arith_op,
    ))
}

/// Compile the kernel text into a JIT engine.
///
/// Failure modes on this path:
///   * Module parse failure → returns `None` and emits a tracing warn.
///     Almost certainly a bug in `build_binop_kernel_text`.
///   * Verifier rejection → same.
///   * `convert-to-llvm` lowering pass failure → likely a dialect op
///     that the umbrella conversion does not know how to lower.
///   * `ExecutionEngine` construction failure → missing LLVM backend.
fn compile_binop_kernel(
    context: &Context,
    op: TensorBinaryOp,
    dtype: DType,
) -> Option<ExecutionEngine> {
    let text = build_binop_kernel_text(op, dtype)?;
    let _location = Location::unknown(context);
    let mut module = match Module::parse(context, &text) {
        Some(m) => m,
        None => {
            tracing::warn!(
                "MLIR-JIT: kernel text parse failed for op={:?} dtype={:?}",
                op,
                dtype
            );
            return None;
        }
    };
    if !module.as_operation().verify() {
        tracing::warn!(
            "MLIR-JIT: kernel verification failed for op={:?} dtype={:?}",
            op,
            dtype
        );
        return None;
    }
    let pass_manager = PassManager::new(context);
    pass_manager.add_pass(pass::conversion::create_to_llvm());
    if pass_manager.run(&mut module).is_err() {
        tracing::warn!(
            "MLIR-JIT: convert-to-llvm pass failed for op={:?} dtype={:?}",
            op,
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
        // Шаг 2 contract: only F32/F64 wired; integer dtypes still
        // route through `CpuBackend` via the dispatcher fall-through.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::I32).unwrap();
        assert!(backend.binop(&a, &b, TensorBinaryOp::Add).is_none());
    }

    #[test]
    fn binop_returns_none_for_unsupported_ops() {
        // Pow / Mod / Min / Max — Шаг 2d wiring points.  Until they
        // land, the dispatcher must fall through to the CPU SIMD
        // ladder for these ops.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[4], DType::F32).unwrap();
        assert!(backend.binop(&a, &b, TensorBinaryOp::Pow).is_none());
        assert!(backend.binop(&a, &b, TensorBinaryOp::Mod).is_none());
        assert!(backend.binop(&a, &b, TensorBinaryOp::Min).is_none());
        assert!(backend.binop(&a, &b, TensorBinaryOp::Max).is_none());
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
    fn unop_reduce_matmul_still_return_none() {
        // Шагов 3/4/5 contract: until those land, the corresponding
        // ops return None so the dispatcher falls through to
        // `CpuBackend`.  This pin guards against accidentally enabling
        // half-wired arms.
        let backend = MlirJitBackend::new();
        let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
        let b = TensorHandle::zeros(&[2], DType::F32).unwrap();
        assert!(backend.unop(&a, TensorUnaryOp::Neg).is_none());
        assert!(backend.reduce(&a, TensorReduceOp::Sum, None).is_none());
        assert!(backend.matmul(&a, &b).is_none());
    }
}
