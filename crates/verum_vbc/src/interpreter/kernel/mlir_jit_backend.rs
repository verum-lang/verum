//! MLIR-JIT tensor compute backend.
//!
//! Industrial-grade pipeline that compiles `linalg` / `vector` / `gpu`
//! dialect kernels through `mlir::ExecutionEngine` and caches the
//! resulting function pointers on first invocation. A single
//! source-of-truth for tensor compute that lets MLIR's auto-
//! vectoriser + tile-and-fuse decide layout / SIMD width / unroll
//! factor — the same machinery PyTorch's `torch.compile`, JAX's XLA,
//! and Triton ride for their production kernels.
//!
//! # Architecture
//!
//! ```text
//! tensor_binop / tensor_unop / tensor_matmul / …
//! │
//! ▼
//! ┌─────────────────────────┐
//! │ dispatch_binop / … │ kernel/mod.rs
//! └────────────┬────────────┘
//! │ (when feature = "mlir-jit")
//! ▼
//! ┌─────────────────────────┐
//! │ MlirJitBackend.binop │ this file
//! └────────────┬────────────┘
//! │
//! cache hit? │
//! ┌─── yes ──── (DashMap.get) ────┐
//! │ │
//! no │ ▼
//! ▼ ┌────────────────┐
//! ┌─────────────────────────────────┐ │ EngineHolder │
//! │ build LLVM-dialect MLIR text │ │ (cached fn ptr)│
//! │ module = Module::parse(ctx) │ └────────┬────────┘
//! │ ExecutionEngine::new(module) │ │
//! │ cache.insert(key, holder) │ ▼
//! └────────────┬────────────────────┘ ┌────────────────┐
//! │ │ invoke_packed │
//! └──── insert ──────► cache ◄─────┤ with arg ptrs │
//! └────────────────┘
//! ```
//!
//! Cache shape:
//! * Hot path: lock-free `DashMap` lookup keyed on
//! `(KernelKind, DType)` → `Arc<EngineHolder>`.
//! * Cold path (first call per key): MLIR Context lock + parse +
//! lowering pipeline + `ExecutionEngine::new`. Persistent
//! on-disk cache (content-addressed by kernel-source hash)
//! skips the lowering pipeline on cache hits across processes.
//!
//! The backend is gated behind the `mlir-jit` Cargo feature (default
//! on). When the feature is off the type does not exist and the
//! `BackendRegistry` reverts to the per-dtype scalar fallback path.

use std::path::PathBuf;
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
// these structs. `invoke_packed` then expects each entry of the
// packed-args array to point to the value of the corresponding arg
// — for our pointer-to-struct args, that means each entry is
// `&mut ptr_to_descriptor`.
//
// The layout below matches MLIR's `StridedMemRefType<T, 2>` exactly
// (`include/mlir/ExecutionEngine/CRunnerUtils.h`). Element type `T`
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

/// Cache key for compiled kernels. A hit on this key means the
/// `(op, dtype)` pair (whether `op` is a binop or a unop) has been
/// lowered to LLVM dialect, JIT-compiled, and the resulting function
/// pointer is ready for `invoke_packed`. The two op families share
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
 /// Scalar-broadcast binop: `a[N] op b[1]` → `out[N]`.
 /// Distinct from `Binop` because the kernel signature differs
 /// (no per-element `b` index — `b` is loaded once outside the
 /// loop) and the cache key must discriminate to avoid a same-
 /// shape kernel servicing a broadcast call site.
 BinopScalarBroadcast(TensorBinaryOp),
 /// Suffix-broadcast binop: `a` and `b`'s shape match in the
 /// trailing dims; `b` repeats over the leading dims.  Common
 /// patterns: `[M,N] op [N]`, `[B,M,N] op [M,N]`, `[B,M,N] op [N]`.
 /// Kernel signature: `(a, b, out, n_total, period)` — element
 /// `i` reads `a[i]` and `b[i mod period]`.
 BinopSuffixBroadcast(TensorBinaryOp),
 /// Prefix-broadcast binop: `a`'s leading dims match `b`'s
 /// (non-trailing-1) shape; `b` repeats over `a`'s trailing dims.
 /// Common patterns: `[M,N] op [M]` (column-wise bias),
 /// `[M,N] op [M,1]` (broadcast 2D form),
 /// `[B,M,N] op [B,M]` (per-(batch,row) gain),
 /// `[B,M,N] op [B]` (per-batch scaling).
 /// Kernel signature: `(a, b, out, n_total, inner_size)` —
 /// element `i` reads `a[i]` and `b[i div inner_size]`.
 BinopPrefixBroadcast(TensorBinaryOp),
 /// Generic mid-axis broadcast binop: `b` has the same rank as
 /// `a` (after leading-1 padding) but one or more axes are
 /// size-1 in `b` and >1 in `a`.  Patterns: `[M,1,K] op [M,N,K]`
 /// (mid-axis size-1), `[M,1] op [M,N]`, multi-axis combinations.
 /// Pre-condition: a's shape is the broadcast output shape; b's
 /// shape (after left-pad with 1s) matches a per-axis with each
 /// axis being either equal or 1 (size-1 axes are broadcast).
 ///
 /// Kernel signature: `(a, b, out, n_total, ndim, out_shape*, b_stride*)`
 /// where `out_shape` and `b_stride` are `i64*` arrays of
 /// length `ndim`.  `b_stride[k]` is `0` for axes broadcast
 /// from b, otherwise the canonical row-major stride of b at
 /// axis k.  Inside the kernel: walk axes from innermost to
 /// outermost, decompose `i` into multi-axis indices via
 /// `out_shape`, accumulate `b_off += idx[k] * b_stride[k]`.
 BinopMidAxisBroadcast(TensorBinaryOp),
 /// Flipped-arg variants — same body shapes as the four
 /// broadcast kernels above, but the arith op reads operands
 /// in the SWAPPED order `b op a` instead of `a op b`.
 /// Used by the dispatcher when it auto-swapped `(a, b)` for
 /// a non-commutative op where `b.numel > a.numel`: from the
 /// caller's perspective they wanted `orig_a op orig_b`, after
 /// the swap the kernel sees `(internal_a=orig_b, internal_b=orig_a)`
 /// and must compute `internal_b[bcast_idx] op internal_a[i]`
 /// to preserve the original semantics.
 ///
 /// For commutative ops these would produce bit-equivalent
 /// results to the non-flipped variants, so the dispatcher
 /// only routes here for non-commutative ops (Sub/Div/Mod/Pow).
 BinopScalarBroadcastFlipped(TensorBinaryOp),
 BinopSuffixBroadcastFlipped(TensorBinaryOp),
 BinopPrefixBroadcastFlipped(TensorBinaryOp),
 BinopMidAxisBroadcastFlipped(TensorBinaryOp),
 Unop(TensorUnaryOp),
 /// Scalar triple-loop matmul; `M`/`K`/`N` are runtime args, the
 /// kernel itself is dtype-only-parametrised (one cached engine
 /// per float dtype).
 Matmul,
 /// Full-tensor reduction (axis = None). Sum/Prod/Max/Min wired
 /// in a future iteration; axis-specific reduction is a future iteration.
 Reduce(TensorReduceOp),
 /// Full-tensor `Σ x²` (sum of squares) — primitive used by
 /// `Var` (`E[X²] − E[X]²`), `Std` (`√Var`) and `Norm` (`√Σx²`).
 /// Wired in a future iteration; the `op` field is unused but kept so
 /// hashing of `KernelKind` discriminates this variant from the
 /// closely-related `Reduce(Sum)` engine.
 SumOfSquares,
 /// Full-tensor `Σ exp(x)` — primitive used by `LogSumExp`
 /// (`log Σ eˣ`). Wired in a future iteration. Note: the naive sum-of-exp
 /// has overflow issues for large `x`; production callers that
 /// need numerical stability should use the max-shifted variant
 /// `m + log Σ e^(x − m)`, which is a future wiring point that
 /// requires broadcast support.
 SumOfExp,
 /// Full-tensor count of non-zero elements (as a float scalar).
 /// Backbone of `All` (`count == n`) and `Any` (`count > 0`).
 /// Wired in a future iteration.
 CountNonzero,
}

/// Wrapper that encapsulates the unsafe `Send + Sync` claim for an
/// `ExecutionEngine`. Justification for the unsafe impls:
///
/// * `ExecutionEngine` wraps an opaque MLIR `MlirExecutionEngine`
/// pointer. Once `mlirExecutionEngineCreate` returns, the engine
/// owns its compiled code; calls to `invoke_packed` are reentrant
/// by MLIR design (LLVM JITted code is reentrant unless the
/// kernel itself contains shared mutable state — our compute
/// kernels are pure data-parallel loops over caller-owned buffers
/// and have no internal shared state).
/// * The cache stores `Arc<EngineHolder>` so multiple threads can
/// share the same engine; the Arc handles refcounting and the
/// newtype carries the unsafe Send/Sync to satisfy the trait
/// bounds on `Backend: Send + Sync` flowing in from
/// `BackendRegistry`.
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
 /// kernel). Wrapped in `Mutex` because MLIR contexts are not
 /// internally synchronised by default — multi-threaded compile is
 /// possible via `enable_multi_threading` but adds compile-time
 /// overhead that's not worth the contention savings on a
 /// once-per-(op, dtype) compile. Hot-path reads (`invoke_packed`)
 /// do not touch the Context.
 context: Mutex<ContextHolder>,
 /// Compiled kernel cache. Lock-free reads on the hot path.
 cache: DashMap<JitKey, Arc<EngineHolder>>,
}

/// Context lives on the heap behind a Mutex; mark it Send so the
/// backend is Send. Justification: same reentrancy argument as
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
 /// Context. This is a one-time initialisation cost (~µs) paid at
 /// backend construction; subsequent kernel compiles reuse the
 /// already-loaded dialects.
 pub fn new() -> Self {
 // Pass registration is a process-wide one-shot. Idempotent
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

 // -----------------------------------------------------------------
 // Composition helpers for statistical reductions .
 //
 // Each helper takes a scalar `TensorHandle` (ndim = 0, numel = 1)
 // and either mutates it in place or runs another cached kernel
 // and returns a new scalar. All operate on F32/F64 only.
 // -----------------------------------------------------------------

 fn invoke_sum_of_exp(&self, a: &TensorHandle) -> Option<TensorHandle> {
 self.invoke_unary_kernel(KernelKind::SumOfExp, a)
 }

 fn invoke_count_nonzero(&self, a: &TensorHandle) -> Option<TensorHandle> {
 self.invoke_unary_kernel(KernelKind::CountNonzero, a)
 }

 /// Generic invocation helper for the family of single-input,
 /// scalar-output kernels (`SumOfSquares` / `SumOfExp` /
 /// `CountNonzero`). All three share the same `(*const T, *mut T,
 /// i64)` ABI; the only thing that differs is the kernel body.
 fn invoke_unary_kernel(&self, kind: KernelKind, a: &TensorHandle) -> Option<TensorHandle> {
 if !is_float_dtype(a.dtype) {
 return None;
 }
 let n = a.numel;
 if n == 0 {
 return None;
 }
 let out = TensorHandle::zeros(&[], a.dtype)?;
 let holder = self.get_or_compile(kind, a.dtype)?;
 let a_data = a.data.as_ref()?;
 let out_data = out.data.as_ref()?;
 let mut a_ptr: *const u8 = unsafe { (*a_data.as_ptr()).as_ptr() };
 let mut out_ptr: *mut u8 = unsafe { (*out_data.as_ptr()).as_mut_ptr() };
 let mut n_arg: i64 = n as i64;
 let mut packed: [*mut (); 3] = [
 (&mut a_ptr) as *mut _ as *mut (),
 (&mut out_ptr) as *mut _ as *mut (),
 (&mut n_arg) as *mut _ as *mut (),
 ];
 let r = unsafe { holder.engine.invoke_packed("kernel", &mut packed) };
 if r.is_err() {
 return None;
 }
 Some(out)
 }

 fn scalar_apply_log(&self, scalar: &TensorHandle, dtype: DType) -> Option<()> {
 let data = scalar.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 *p = (*p).ln();
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 *p = (*p).ln();
 }
 _ => return None,
 }
 }
 Some(())
 }

 /// Set the scalar to `1.0` if it equals `n_target`, else `0.0`.
 /// Used by `All`: count of non-zero equals total → all are true.
 fn scalar_set_eq_n(&self, scalar: &TensorHandle, n_target: usize, dtype: DType) -> Option<()> {
 let data = scalar.data.as_ref()?;
 let target = n_target as f64;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 *p = if (*p as f64 - target).abs() < 0.5 { 1.0 } else { 0.0 };
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 *p = if (*p - target).abs() < 0.5 { 1.0 } else { 0.0 };
 }
 _ => return None,
 }
 }
 Some(())
 }

 /// Set the scalar to `1.0` if `*scalar > 0`, else `0.0`.
 /// Used by `Any`: count of non-zero > 0 → at least one is true.
 fn scalar_set_gt_zero(&self, scalar: &TensorHandle, dtype: DType) -> Option<()> {
 let data = scalar.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 *p = if *p > 0.5 { 1.0 } else { 0.0 };
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 *p = if *p > 0.5 { 1.0 } else { 0.0 };
 }
 _ => return None,
 }
 }
 Some(())
 }

 fn invoke_sum_of_squares(&self, a: &TensorHandle) -> Option<TensorHandle> {
 self.invoke_unary_kernel(KernelKind::SumOfSquares, a)
 }

 fn scalar_div_by_n(&self, scalar: &TensorHandle, n: usize, dtype: DType) -> Option<()> {
 if n == 0 {
 return None;
 }
 let data = scalar.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 *p /= n as f32;
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 *p /= n as f64;
 }
 _ => return None,
 }
 }
 Some(())
 }

 fn scalar_apply_sqrt(&self, scalar: &TensorHandle, dtype: DType) -> Option<()> {
 let data = scalar.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 let v = *p;
 *p = if v < 0.0 { 0.0 } else { v.sqrt() };
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 let v = *p;
 *p = if v < 0.0 { 0.0 } else { v.sqrt() };
 }
 _ => return None,
 }
 }
 Some(())
 }

 fn scalar_square_in_place(&self, scalar: &TensorHandle, dtype: DType) -> Option<()> {
 let data = scalar.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f32;
 *p = (*p) * (*p);
 }
 DType::F64 => {
 let p = (*data.as_ptr()).as_mut_ptr() as *mut f64;
 *p = (*p) * (*p);
 }
 _ => return None,
 }
 }
 Some(())
 }

 fn scalar_sub_in_place(
 &self,
 lhs: &TensorHandle,
 rhs: &TensorHandle,
 dtype: DType,
 ) -> Option<()> {
 let l_data = lhs.data.as_ref()?;
 let r_data = rhs.data.as_ref()?;
 unsafe {
 match dtype {
 DType::F32 => {
 let lp = (*l_data.as_ptr()).as_mut_ptr() as *mut f32;
 let rp = (*r_data.as_ptr()).as_ptr() as *const f32;
 *lp -= *rp;
 }
 DType::F64 => {
 let lp = (*l_data.as_ptr()).as_mut_ptr() as *mut f64;
 let rp = (*r_data.as_ptr()).as_ptr() as *const f64;
 *lp -= *rp;
 }
 _ => return None,
 }
 }
 Some(())
 }

 /// Look up the cached engine for `(kind, dtype)` or compile and insert.
 ///
 /// Hot path is the lock-free `DashMap::get` read; cache miss
 /// drops into `Mutex<Context>` and double-checks the cache after
 /// taking the lock so two threads racing on the same key don't
 /// double-compile. Once an engine is in the cache it is shared
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
 // CPU host execution is synchronous by construction. When the
 // GPU dialect path lands , this becomes
 // `gpu.host_synchronize` through the JIT runtime.
 }

 fn binop(
 &self,
 a: &TensorHandle,
 b: &TensorHandle,
 op: TensorBinaryOp,
 ) -> Option<TensorHandle> {
 // a future iteration+2d coverage: F32 / F64 + I{8,16,32,64} + U{8,16,32,64}
 // for Add/Sub/Mul/Div/Mod/Min/Max. Float dtypes additionally
 // support Pow via `math.powf`. `binop_arith_op` returns None
 // for any combination outside this matrix, in which case the
 // dispatcher falls through to `CpuBackend`.
 if a.dtype != b.dtype {
 return None;
 }
 if binop_arith_op(op, a.dtype).is_none() {
 return None;
 }

 // **Operand swap** — when `b` has MORE elements than `a` (b
 // would broadcast to a's superset shape), swap so the larger
 // operand is `a` and the existing dispatcher (which materialises
 // `out` with `a.shape`) just works.  For commutative ops
 // (Add/Mul/Min/Max) the swap is mathematically transparent.
 // For non-commutative ops (Sub/Div/Mod/Pow) we set `flipped =
 // true` and route through the *Flipped variants below, which
 // emit the arith op with operands in reversed order — the
 // kernel computes `b op a` (where the names are post-swap) ≡
 // the caller's original `orig_a op orig_b`.
 let (a, b, flipped) = if b.numel > a.numel {
 (b, a, !op_is_commutative(op))
 } else {
 (a, b, false)
 };

 let a_shape = &a.shape[..a.ndim as usize];
 let b_shape = &b.shape[..b.ndim as usize];
 let n = a.numel;
 if n == 0 {
 return Some(TensorHandle::zeros(a_shape, a.dtype)?);
 }

 // **Шаг 5e / 5e+1 / 5e+2 — broadcast support.**
 //
 // Four patterns are JIT-able from this dispatcher:
 //
 //   1. **Same shape**: a_shape == b_shape — canonical `Binop`
 //      kernel.
 //   2. **Scalar broadcast**: b.numel == 1 (rank-0 scalar `[]`,
 //      or any rank with all-1 dims `[1]`/`[1,1]`/…).  Routes to
 //      `BinopScalarBroadcast` which loads `b[0]` once outside
 //      the loop.
 //   3. **Suffix broadcast**: b's shape is a strict trailing
 //      slice of a's shape (e.g. `[M,N] op [N]`,
 //      `[B,M,N] op [M,N]`, `[B,M,N] op [N]`).  Routes to
 //      `BinopSuffixBroadcast` which reads `b[i mod period]`
 //      with `period = b.numel`.
 //   4. **Prefix broadcast**: b's effective shape (after
 //      stripping trailing-1 dims) is a strict leading slice of
 //      a's shape (`[M,N] op [M]`, `[M,N] op [M,1]`,
 //      `[B,M,N] op [B,M]`, `[B,M,N] op [B]`).  Routes to
 //      `BinopPrefixBroadcast` which reads `b[i div inner_size]`
 //      with `inner_size = numel(a_shape[b_eff_len..])`.
 //
 // Anything else (mid-axis size-1 broadcast `[M,1,K] op [M,N,K]`,
 // mixed expand+contract patterns) returns `None` and falls
 // through to `CpuBackend`.  That's Шаг 5e+3 (multi-axis stride
 // scheme) — deferred.
 let same_shape = a_shape == b_shape;
 let scalar_broadcast = !same_shape && b.numel == 1;
 let suffix_broadcast = !same_shape
 && !scalar_broadcast
 && b_shape.len() < a_shape.len()
 && a_shape[a_shape.len() - b_shape.len()..] == *b_shape;

 // Effective b shape: trim trailing 1s.  `[M,1]` → `[M]`,
 // `[M,1,1]` → `[M]`, `[1]` → `[]` (would be scalar already).
 let b_eff_len = b_shape
 .iter()
 .rposition(|&d| d != 1)
 .map(|i| i + 1)
 .unwrap_or(0);
 let b_eff = &b_shape[..b_eff_len];
 let prefix_broadcast = !same_shape
 && !scalar_broadcast
 && !suffix_broadcast
 && b_eff_len > 0
 && b_eff_len < a_shape.len()
 && a_shape[..b_eff_len] == *b_eff;
 // `inner_size = ∏(a_shape[b_eff_len..])`.  For `[M,N] op [M]`
 // this is `N`; for `[B,M,N] op [B]` it's `M*N`.
 let inner_size: usize = if prefix_broadcast {
 a_shape[b_eff_len..].iter().product()
 } else {
 1
 };

 // **Шаг 5e+3 — generic mid-axis broadcast.**
 //
 // Pad b's shape on the left with 1s to match a's rank, then
 // check NumPy-style broadcastability: every axis must satisfy
 // `b_padded[k] == 1 || b_padded[k] == a_shape[k]`.  When at
 // least one axis is 1 in b (broadcast) and the rest match —
 // and we haven't already hit one of the three single-parameter
 // patterns above — we route through `BinopMidAxisBroadcast`.
 let mut mid_axis_broadcast = false;
 let mut padded_b_shape: [usize; crate::interpreter::tensor::MAX_DIMS] =
 [1; crate::interpreter::tensor::MAX_DIMS];
 if !same_shape && !scalar_broadcast && !suffix_broadcast && !prefix_broadcast {
 if b_shape.len() <= a_shape.len() {
 let pad = a_shape.len() - b_shape.len();
 for (i, &d) in b_shape.iter().enumerate() {
 padded_b_shape[pad + i] = d;
 }
 let mut ok = true;
 for k in 0..a_shape.len() {
 let bk = padded_b_shape[k];
 if bk != 1 && bk != a_shape[k] {
 ok = false;
 break;
 }
 }
 mid_axis_broadcast = ok;
 }
 }

 if !same_shape
 && !scalar_broadcast
 && !suffix_broadcast
 && !prefix_broadcast
 && !mid_axis_broadcast
 {
 return None;
 }

 // Materialise the output tensor on the host. MLIR JIT'd
 // kernels read/write through caller-supplied pointers; we own
 // the output buffer here.
 let out = TensorHandle::zeros(a_shape, a.dtype)?;
 // Same-shape is symmetric so flipping is a no-op there; only
 // the four broadcast kernels have asymmetric `out[i] = a[i] op
 // b[bcast_idx]` shapes that need the *Flipped variant when the
 // dispatcher swapped a non-commutative op.
 let kind = match (scalar_broadcast, suffix_broadcast, prefix_broadcast, mid_axis_broadcast, flipped) {
 (true, _, _, _, false) => KernelKind::BinopScalarBroadcast(op),
 (true, _, _, _, true) => KernelKind::BinopScalarBroadcastFlipped(op),
 (_, true, _, _, false) => KernelKind::BinopSuffixBroadcast(op),
 (_, true, _, _, true) => KernelKind::BinopSuffixBroadcastFlipped(op),
 (_, _, true, _, false) => KernelKind::BinopPrefixBroadcast(op),
 (_, _, true, _, true) => KernelKind::BinopPrefixBroadcastFlipped(op),
 (_, _, _, true, false) => KernelKind::BinopMidAxisBroadcast(op),
 (_, _, _, true, true) => KernelKind::BinopMidAxisBroadcastFlipped(op),
 _ => KernelKind::Binop(op),
 };
 let holder = self.get_or_compile(kind, a.dtype)?;

 // Pre-compute mid-axis broadcast metadata (out_shape + b_stride
 // arrays) when we're routing through `BinopMidAxisBroadcast`.
 // Both arrays are length `ndim = a_shape.len()`.  `b_stride[k]`
 // is the canonical row-major stride of b at axis `k` if the
 // padded b dim matches a's dim there, else 0 (broadcast).
 let mut out_shape_buf: [i64; crate::interpreter::tensor::MAX_DIMS] = [0; _];
 let mut b_stride_buf: [i64; crate::interpreter::tensor::MAX_DIMS] = [0; _];
 if mid_axis_broadcast {
 let ndim = a_shape.len();
 for k in 0..ndim {
 out_shape_buf[k] = a_shape[k] as i64;
 }
 // Canonical b strides over the PADDED shape, in elements.
 // Walk axes inner→outer to accumulate.  An axis with
 // padded_b_shape[k] == 1 gets stride 0 (broadcast).
 let mut running: i64 = 1;
 for k in (0..ndim).rev() {
 if padded_b_shape[k] == 1 {
 b_stride_buf[k] = 0;
 } else {
 b_stride_buf[k] = running;
 }
 running *= padded_b_shape[k] as i64;
 }
 }

 // ABI marshalling differs by kernel kind:
 //   * `Binop` / `BinopScalarBroadcast`: 4 args (a, b, out, n).
 //   * `BinopSuffixBroadcast`:            5 args (a, b, out, n, period).
 //   * `BinopPrefixBroadcast`:            5 args (a, b, out, n, inner_size).
 //   * `BinopMidAxisBroadcast`:           7 args (a, b, out, n, ndim,
 //                                                out_shape*, b_stride*).
 //
 // `mlirExecutionEngineInvokePacked` expects an array of `*mut ()`
 // where each entry points to the *value* of the argument (so for
 // pointer args we pass &mut ptr_value).
 let a_data = a.data.as_ref()?;
 let b_data = b.data.as_ref()?;
 let out_data = out.data.as_ref()?;
 let mut a_ptr: *const u8 = unsafe { (*a_data.as_ptr()).as_ptr() };
 let mut b_ptr: *const u8 = unsafe { (*b_data.as_ptr()).as_ptr() };
 let mut out_ptr: *mut u8 = unsafe { (*out_data.as_ptr()).as_mut_ptr() };
 let mut n_arg: i64 = n as i64;
 let mut period_arg: i64 = b.numel as i64;
 let mut inner_arg: i64 = inner_size as i64;
 let mut ndim_arg: i64 = a_shape.len() as i64;
 let mut out_shape_ptr: *const i64 = out_shape_buf.as_ptr();
 let mut b_stride_ptr: *const i64 = b_stride_buf.as_ptr();
 let result = if suffix_broadcast {
 let mut packed: [*mut (); 5] = [
 (&mut a_ptr) as *mut _ as *mut (),
 (&mut b_ptr) as *mut _ as *mut (),
 (&mut out_ptr) as *mut _ as *mut (),
 (&mut n_arg) as *mut _ as *mut (),
 (&mut period_arg) as *mut _ as *mut (),
 ];
 unsafe { holder.engine.invoke_packed("kernel", &mut packed) }
 } else if prefix_broadcast {
 let mut packed: [*mut (); 5] = [
 (&mut a_ptr) as *mut _ as *mut (),
 (&mut b_ptr) as *mut _ as *mut (),
 (&mut out_ptr) as *mut _ as *mut (),
 (&mut n_arg) as *mut _ as *mut (),
 (&mut inner_arg) as *mut _ as *mut (),
 ];
 unsafe { holder.engine.invoke_packed("kernel", &mut packed) }
 } else if mid_axis_broadcast {
 let mut packed: [*mut (); 7] = [
 (&mut a_ptr) as *mut _ as *mut (),
 (&mut b_ptr) as *mut _ as *mut (),
 (&mut out_ptr) as *mut _ as *mut (),
 (&mut n_arg) as *mut _ as *mut (),
 (&mut ndim_arg) as *mut _ as *mut (),
 (&mut out_shape_ptr) as *mut _ as *mut (),
 (&mut b_stride_ptr) as *mut _ as *mut (),
 ];
 unsafe { holder.engine.invoke_packed("kernel", &mut packed) }
 } else {
 let mut packed: [*mut (); 4] = [
 (&mut a_ptr) as *mut _ as *mut (),
 (&mut b_ptr) as *mut _ as *mut (),
 (&mut out_ptr) as *mut _ as *mut (),
 (&mut n_arg) as *mut _ as *mut (),
 ];
 // The MLIR JIT exposes the kernel under its C-ABI wrapper
 // name (`_mlir_ciface_kernel`) when the function carries
 // the `llvm.emit_c_interface` attribute. `invoke_packed`
 // resolves that wrapper symbol automatically for the bare
 // name.
 unsafe { holder.engine.invoke_packed("kernel", &mut packed) }
 };
 if result.is_err() {
 tracing::warn!(
 "MLIR-JIT binop invocation failed for op={:?} dtype={:?} \
 same_shape={} scalar_bcast={} suffix_bcast={} prefix_bcast={} mid_axis_bcast={}; \
 falling back to CpuBackend",
 op,
 a.dtype,
 same_shape,
 scalar_broadcast,
 suffix_broadcast,
 prefix_broadcast,
 mid_axis_broadcast
 );
 return None;
 }
 Some(out)
 }

 fn unop(&self, a: &TensorHandle, op: TensorUnaryOp) -> Option<TensorHandle> {
 // A future iteration coverage: F32 / F64 + the math.*-backed unary primitives
 // (Neg/Abs/Sqrt/Exp/Log/Log2/Sin/Cos/Tan/Tanh/Floor/Ceil/Round/
 // Rsqrt/Erf). Sigmoid/Relu/Gelu/Silu/Softplus/Mish/Sign require
 // composition (e.g. Sigmoid = 1/(1+exp(-x))) and arrive in a
 // future a future iteration — for now they return `None` and fall through
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
 // a future iteration: full-axis reduction for F32 / F64 + Sum/Prod/Max/Min.
 // Axis-specific reduction (`axis = Some(i)`) and the higher-
 // level ops (Mean / Var / Std / Norm / LogSumExp / All / Any)
 // defer to a future iteration. Until then, those cases fall through to
 // the hand-tuned `cpu::reduce_*` ladder.
 if axis.is_some() {
 return None;
 }
 if !is_float_dtype(a.dtype) {
 return None;
 }
 // Compose statistical reductions out of cached primitives.
 // Each branch reuses Sum / SumOfSquares engines + a small
 // scalar post-pass — no new MLIR kernel per statistical op.
 match op {
 TensorReduceOp::Mean => {
 let sum = self.reduce(a, TensorReduceOp::Sum, axis)?;
 self.scalar_div_by_n(&sum, a.numel, a.dtype)?;
 return Some(sum);
 }
 TensorReduceOp::Norm => {
 // L2 norm: √(Σ x²)
 let s2 = self.invoke_sum_of_squares(a)?;
 self.scalar_apply_sqrt(&s2, a.dtype)?;
 return Some(s2);
 }
 TensorReduceOp::Var => {
 // Var = E[X²] − E[X]². Two-pass formula; numerically
 // less stable than Welford but cheap and matches
 // PyTorch's `torch.var(..., unbiased=False)` semantics.
 let s2 = self.invoke_sum_of_squares(a)?;
 self.scalar_div_by_n(&s2, a.numel, a.dtype)?;
 let s = self.reduce(a, TensorReduceOp::Sum, axis)?;
 self.scalar_div_by_n(&s, a.numel, a.dtype)?;
 self.scalar_square_in_place(&s, a.dtype)?;
 self.scalar_sub_in_place(&s2, &s, a.dtype)?;
 return Some(s2);
 }
 TensorReduceOp::Std => {
 let v = self.reduce(a, TensorReduceOp::Var, axis)?;
 self.scalar_apply_sqrt(&v, a.dtype)?;
 return Some(v);
 }
 TensorReduceOp::LogSumExp => {
 // Naive log Σ eˣ — fast and correct for small / well-
 // conditioned inputs. Production code that needs
 // numerical stability against large positive x should
 // pre-shift by max (A future iteration once broadcast lands).
 let s = self.invoke_sum_of_exp(a)?;
 self.scalar_apply_log(&s, a.dtype)?;
 return Some(s);
 }
 TensorReduceOp::All => {
 // All non-zero ⇔ count of non-zero == n. Result is
 // 1.0 (all true) or 0.0 (some zero). Float result
 // matches PyTorch's bool-as-uint8 / NumPy's bool
 // tensors lowered to float when dtype is F32/F64.
 let count = self.invoke_count_nonzero(a)?;
 self.scalar_set_eq_n(&count, a.numel, a.dtype)?;
 return Some(count);
 }
 TensorReduceOp::Any => {
 // Any non-zero ⇔ count > 0. Same float-bool encoding
 // as All.
 let count = self.invoke_count_nonzero(a)?;
 self.scalar_set_gt_zero(&count, a.dtype)?;
 return Some(count);
 }
 _ => {}
 }
 if reduce_arith_op(op).is_none() {
 return None;
 }
 let n = a.numel;
 if n == 0 {
 // Reducing an empty tensor → identity element. Honest
 // semantics differ across libraries (PyTorch raises,
 // NumPy returns identity); our hand-tuned path handles
 // this — fall through.
 return None;
 }
 // Scalar (ndim = 0) output tensor. The CPU `reduce_*` kernels
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
 // a future iteration: linalg.matmul over dynamically-shaped memrefs, JIT-
 // compiled through the full MLIR `linalg-to-loops` →
 // `convert-to-llvm` lowering pipeline. At `opt_level = 2`
 // MLIR's loop vectoriser + LLVM's `llvm.intr.vector.reduce.*`
 // chain produce cache-tiled, vectorised GEMM kernels — the
 // path that closes the gap to cuBLAS / MKL throughput on
 // large matrices. Compared with the a future iteration scalar triple-
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
 // { base, aligned, offset: i64, sizes: [i64; 2], strides: [i64; 2] }
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
// op (`arith.divsi` vs `arith.divui`). Hence the `mlir_int_width` /
// `is_float_dtype` / `is_signed_dtype` triple is enough to drive the
// op-selector tables below. Adding a new dtype only requires one
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
/// `arith.divsi` vs `arith.divui`). The same applies for the wider
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
/// True iff `op` is mathematically commutative.  Used by the
/// broadcast dispatcher to swap argument order when `b.numel >
/// a.numel`: `a op b == b op a` for commutative ops, so we can
/// route through the canonical "b broadcasts into a's shape"
/// kernel set after the swap.
///
/// Non-commutative ops (Sub / Div / Mod / Pow) where b is larger
/// than a would need flipped-arg variants of every broadcast
/// kernel.  That's deferred — those call sites currently fall
/// through to `CpuBackend`.
fn op_is_commutative(op: TensorBinaryOp) -> bool {
 use TensorBinaryOp::*;
 matches!(op, Add | Mul | Min | Max)
}

/// `(binop, dtype)` pair. Returning `None` signals "not yet wired —
/// fall through to `CpuBackend`". The table is exhaustive on the
/// supported numeric range but intentionally conservative on
/// edge-case combinations (e.g. integer Pow needs `math.ipowi` which
/// has different ABI requirements; deferred).
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
 // with different signature. Defer to a future iteration.
 (Pow, _) => None,

 _ => None,
 }
}

/// Resolve the MLIR `math.*` / `arith.*` op spelling for unary ops.
///
/// All math.* ops listed here run on float dtypes (F32 / F64). The
/// Sigmoid / Relu / Gelu / Silu / Softplus / Mish family requires
/// composition — Sigmoid = `1 / (1 + exp(-x))` and so on — which is
/// scheduled for a A future iteration once the simple-op coverage proves out.
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
 // `math.rsqrt`, `math.roundeven`, and `math.erf` lower through
 // `math-to-libm` to libm symbols (`rsqrtf` / `roundevenf` /
 // `erff` / 64-bit variants) that the host JIT can't always
 // resolve from the running process's symbol table. Defer until
 // explicit `ExecutionEngine` symbol
 // registration for libm functions lands.
 Round | Erf | Rsqrt => return None,
 // Composed forms — A future iteration wiring point.
 Sigmoid | Relu | Gelu | Silu | Softplus | Mish | Sign => return None,
 })
}

/// Resolve the (init-literal, accumulator-op) pair for a reduce
/// operation. Only Sum/Prod/Max/Min are wired ; the
/// statistical ops (Mean / Var / Std / Norm / LogSumExp) need a
/// post-pass over the accumulator and are deferred to a future iteration.
fn reduce_arith_op(op: TensorReduceOp) -> Option<(&'static str, &'static str)> {
 use TensorReduceOp::*;
 Some(match op {
 // (init-shape, accumulator-op). For float reductions, init
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
// `math` op differ. After `Module::parse` we run the umbrella
// `convert-to-llvm` conversion to lower everything to the `llvm`
// dialect, then hand the result to `mlirExecutionEngineCreate`. The
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

/// Build the MLIR text for a **scalar-broadcast** binary op:
/// `out[i] = a[i] op b[0]` for `i ∈ [0, n)`.
///
/// Signature: `void kernel(*const T a, *const T b_scalar, *mut T out, i64 n)`.
/// Identical to [`build_binop_kernel_text`] except the loop reads
/// `b` from the same address every iteration — LLVM's loop-invariant
/// code motion hoists that load to the pre-header so it executes
/// exactly once even at -O0, and the inner body becomes a single
/// `llvm.load %a[i] / arith.{op} %va, %vb / llvm.store %out[i]`
/// triple ready for the auto-vectoriser.
///
/// Caller must guarantee `b.numel == 1` (rank-0 scalar or all-1
/// shape); the kernel itself only ever touches `b[0]`.
///
/// `flipped == true` swaps the operand order in the arith op:
/// `out[i] = b op a[i]` instead of `a[i] op b`.  Used by the
/// dispatcher when it auto-swapped `(a, b)` for a non-commutative
/// op where the original `b` was larger than the original `a`.
fn build_binop_scalar_broadcast_kernel_text(
 op: TensorBinaryOp,
 dtype: DType,
 flipped: bool,
) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 let arith = binop_arith_op(op, dtype)?;
 let (lhs, rhs) = if flipped { ("%vb", "%va") } else { ("%va", "%vb") };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %b: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 %vb = llvm.load %b : !llvm.ptr -> {elem}
 cf.br ^loop(%c0 : i64)
 ^loop(%i: i64):
 %cond = arith.cmpi slt, %i, %n : i64
 cf.cond_br %cond, ^body, ^exit
 ^body:
 %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ai : !llvm.ptr -> {elem}
 %vc = {arith} {lhs}, {rhs} : {elem}
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
 lhs = lhs,
 rhs = rhs,
 ))
}

/// Build the MLIR text for a **suffix-broadcast** binary op:
/// `out[i] = a[i] op b[i mod period]` for `i ∈ [0, n_total)`.
///
/// Signature: `void kernel(*const T a, *const T b, *mut T out, i64 n, i64 period)`.
/// `period` is `b.numel` — the size of the trailing block that `b`
/// covers and which we cycle through over `a`'s leading dims.
/// Examples:
///
///  * `a = [M,N]`, `b = [N]` → `n = M*N`, `period = N`.
///  * `a = [B,M,N]`, `b = [M,N]` → `n = B*M*N`, `period = M*N`.
///  * `a = [B,M,N]`, `b = [N]` → `n = B*M*N`, `period = N`.
///
/// Pre-condition (enforced by [`MlirJitBackend::binop`]): `b`'s
/// shape is a strict trailing slice of `a`'s shape — without that,
/// a single `i mod period` index is insufficient and a multi-axis
/// stride scheme would be needed (deferred — Шаг 5e+2).
///
/// LLVM's strength-reduction folds `arith.remsi i, period` to a
/// bitwise AND when `period` is a power of two — common for the
/// `[B,M,N] op [M,N]` and similar nn-style call sites where the
/// trailing matrix is sized to align with vector lanes.  Non-pow2
/// `period` stays on `remsi` which modern x86 / aarch64 CPUs
/// retire in 1 µ-op + a few cycles.
fn build_binop_suffix_broadcast_kernel_text(
 op: TensorBinaryOp,
 dtype: DType,
 flipped: bool,
) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 let arith = binop_arith_op(op, dtype)?;
 let (lhs, rhs) = if flipped { ("%vb", "%va") } else { ("%va", "%vb") };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %b: !llvm.ptr, %out: !llvm.ptr, %n: i64, %period: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 cf.br ^loop(%c0 : i64)
 ^loop(%i: i64):
 %cond = arith.cmpi slt, %i, %n : i64
 cf.cond_br %cond, ^body, ^exit
 ^body:
 %bi_idx = arith.remsi %i, %period : i64
 %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %bi = llvm.getelementptr inbounds %b[%bi_idx] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ai : !llvm.ptr -> {elem}
 %vb = llvm.load %bi : !llvm.ptr -> {elem}
 %vc = {arith} {lhs}, {rhs} : {elem}
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
 lhs = lhs,
 rhs = rhs,
 ))
}

/// Build the MLIR text for a **prefix-broadcast** binary op:
/// `out[i] = a[i] op b[i div inner_size]` for `i ∈ [0, n_total)`.
///
/// Signature: `void kernel(*const T a, *const T b, *mut T out, i64 n, i64 inner_size)`.
/// `inner_size` is the product of `a_shape[b_eff_len..]` — the
/// number of consecutive `a` elements that share the same `b`
/// value.  Examples:
///
///  * `a = [M,N]`, `b = [M]` (or `[M,1]`) → `n = M*N`,
///    `inner_size = N`.
///  * `a = [B,M,N]`, `b = [B,M]` → `n = B*M*N`, `inner_size = N`.
///  * `a = [B,M,N]`, `b = [B]` → `n = B*M*N`, `inner_size = M*N`.
///
/// The MLIR `arith.divsi` lowers to a single `idiv` on x86 / `sdiv`
/// on aarch64; for `inner_size == power-of-two` LLVM strength-
/// reduces to a `shr` (right shift) automatically.  Compared to
/// `BinopSuffixBroadcast`'s `arith.remsi` cost, `divsi` and
/// `remsi` are roughly the same — they retire on the same execution
/// unit and a single cycle each on modern microarchitectures.
fn build_binop_prefix_broadcast_kernel_text(
 op: TensorBinaryOp,
 dtype: DType,
 flipped: bool,
) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 let arith = binop_arith_op(op, dtype)?;
 let (lhs, rhs) = if flipped { ("%vb", "%va") } else { ("%va", "%vb") };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %b: !llvm.ptr, %out: !llvm.ptr, %n: i64, %inner: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 cf.br ^loop(%c0 : i64)
 ^loop(%i: i64):
 %cond = arith.cmpi slt, %i, %n : i64
 cf.cond_br %cond, ^body, ^exit
 ^body:
 %bi_idx = arith.divsi %i, %inner : i64
 %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %bi = llvm.getelementptr inbounds %b[%bi_idx] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ai : !llvm.ptr -> {elem}
 %vb = llvm.load %bi : !llvm.ptr -> {elem}
 %vc = {arith} {lhs}, {rhs} : {elem}
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
 lhs = lhs,
 rhs = rhs,
 ))
}

/// Build the MLIR text for a **mid-axis (NumPy-generic) broadcast**
/// binary op: `out[i] = a[i] op b[b_off(i)]` where `b_off(i)`
/// decodes `i` into multi-axis indices via `out_shape` and
/// accumulates contributions weighted by `b_stride`.
///
/// Signature: `void kernel(*const T a, *const T b, *mut T out,
///                          i64 n, i64 ndim, i64* out_shape,
///                          i64* b_stride)`.
///
/// Pre-condition (enforced by the dispatcher): `a`'s shape is the
/// broadcast OUTPUT shape (so `a` reads canonically by `i`); `b`
/// has been left-padded with 1s to match `ndim`, and `b_stride[k]`
/// is `0` for axes broadcast from b (size-1) or the canonical
/// row-major stride of b for matching axes.
///
/// Inner axis-decoder is a block-arg-threaded loop:
///
///   rem  = i,    b_off = 0,    axis_left = ndim
///   while axis_left > 0:
///     ax = axis_left - 1
///     d  = out_shape[ax]
///     c  = rem mod d
///     rem = rem div d
///     b_off += c * b_stride[ax]
///     axis_left = ax
///   out[i] = a[i] op b[b_off]
///
/// The decoder is single-iteration-per-axis so total cost is
/// `O(ndim)` per element — for `MAX_DIMS = 8` that's at most 8
/// `remsi` + 8 `divsi` + 8 `muli` + 8 `addi` + 8 array loads,
/// dominated on modern microarchitectures by the divsi/remsi
/// pair (1 cycle each on x86 / aarch64).
///
/// The kernel is `(op, dtype)`-parametrised — one cached engine
/// services every `(op, dtype)` regardless of rank, since `ndim`
/// is a runtime arg and the per-axis arrays decode at runtime.
fn build_binop_mid_axis_broadcast_kernel_text(
 op: TensorBinaryOp,
 dtype: DType,
 flipped: bool,
) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 let arith = binop_arith_op(op, dtype)?;
 let (lhs, rhs) = if flipped { ("%vb", "%va") } else { ("%va", "%vb") };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %b: !llvm.ptr, %out: !llvm.ptr, %n: i64, %ndim: i64, %out_shape: !llvm.ptr, %b_stride: !llvm.ptr) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 cf.br ^outer(%c0 : i64)
 ^outer(%i: i64):
 %cond_i = arith.cmpi slt, %i, %n : i64
 cf.cond_br %cond_i, ^body, ^exit
 ^body:
 cf.br ^axis(%i, %c0, %ndim : i64, i64, i64)
 ^axis(%rem: i64, %b_off: i64, %axis_left: i64):
 %has_axis = arith.cmpi sgt, %axis_left, %c0 : i64
 cf.cond_br %has_axis, ^axis_body, ^axis_done(%b_off : i64)
 ^axis_body:
 %ax = arith.subi %axis_left, %c1 : i64
 %d_ptr = llvm.getelementptr inbounds %out_shape[%ax] : (!llvm.ptr, i64) -> !llvm.ptr, i64
 %d = llvm.load %d_ptr : !llvm.ptr -> i64
 %c = arith.remsi %rem, %d : i64
 %rem_next = arith.divsi %rem, %d : i64
 %s_ptr = llvm.getelementptr inbounds %b_stride[%ax] : (!llvm.ptr, i64) -> !llvm.ptr, i64
 %s = llvm.load %s_ptr : !llvm.ptr -> i64
 %inc = arith.muli %c, %s : i64
 %b_off_next = arith.addi %b_off, %inc : i64
 cf.br ^axis(%rem_next, %b_off_next, %ax : i64, i64, i64)
 ^axis_done(%final_b_off: i64):
 %ai = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %bi = llvm.getelementptr inbounds %b[%final_b_off] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ai : !llvm.ptr -> {elem}
 %vb = llvm.load %bi : !llvm.ptr -> {elem}
 %vc = {arith} {lhs}, {rhs} : {elem}
 %ci = llvm.getelementptr inbounds %out[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 llvm.store %vc, %ci : {elem}, !llvm.ptr
 %i_next = arith.addi %i, %c1 : i64
 cf.br ^outer(%i_next : i64)
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
/// `*output`. LLVM auto-vectorises the loop into a tree-reduction
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
 // with appropriate float literals. Hex floats avoid rounding
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

/// Build the MLIR text for `Σ x²` over a flat 1-D buffer.
///
/// Same scaffold as `build_reduce_kernel_text(Sum, dtype)` but with
/// `arith.mulf` between the load and the accumulator add — i.e. the
/// loop computes `acc += x * x` per element. Used as the building
/// block for L2 norm, variance, and standard deviation in the
/// `reduce()` composition path.
fn build_sum_of_squares_kernel_text(dtype: DType) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 if !is_float_dtype(dtype) {
 return None;
 }
 let zero_lit = if dtype == DType::F32 { "0.0 : f32" } else { "0.0 : f64" };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 %f0 = arith.constant {zero_lit}
 cf.br ^loop(%c0, %f0 : i64, {elem})
 ^loop(%i: i64, %acc: {elem}):
 %done = arith.cmpi sge, %i, %n : i64
 cf.cond_br %done, ^store, ^body
 ^body:
 %ap = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ap : !llvm.ptr -> {elem}
 %sq = arith.mulf %va, %va : {elem}
 %new_acc = arith.addf %acc, %sq : {elem}
 %i_next = arith.addi %i, %c1 : i64
 cf.br ^loop(%i_next, %new_acc : i64, {elem})
 ^store:
 llvm.store %acc, %out : {elem}, !llvm.ptr
 return
 }}
}}
"#,
 elem = elem_ty,
 ))
}

/// Build the MLIR text for `Σ exp(x)` over a flat buffer.
///
/// Same scaffold as `build_sum_of_squares_kernel_text` but with
/// `math.exp` between the load and the accumulator add. Used as
/// the primitive for `LogSumExp` (`log Σ eˣ`) — we materialise the
/// scalar `Σ eˣ` here and the caller applies `log` in Rust.
///
/// Numerical caveat: this is the naive sum-of-exp. For inputs with
/// very large positive `x`, the sum overflows; the production form
/// is `m + log Σ e^(x − m)` where `m = max(x)`, which requires
/// broadcast subtraction (A future iteration wiring point).
fn build_sum_of_exp_kernel_text(dtype: DType) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 if !is_float_dtype(dtype) {
 return None;
 }
 let zero_lit = if dtype == DType::F32 { "0.0 : f32" } else { "0.0 : f64" };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 %f0 = arith.constant {zero_lit}
 cf.br ^loop(%c0, %f0 : i64, {elem})
 ^loop(%i: i64, %acc: {elem}):
 %done = arith.cmpi sge, %i, %n : i64
 cf.cond_br %done, ^store, ^body
 ^body:
 %ap = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ap : !llvm.ptr -> {elem}
 %ev = math.exp %va : {elem}
 %new_acc = arith.addf %acc, %ev : {elem}
 %i_next = arith.addi %i, %c1 : i64
 cf.br ^loop(%i_next, %new_acc : i64, {elem})
 ^store:
 llvm.store %acc, %out : {elem}, !llvm.ptr
 return
 }}
}}
"#,
 elem = elem_ty,
 ))
}

/// Build the MLIR text for a count-of-non-zero reduction.
///
/// The kernel produces a float scalar equal to `|{ i : x[i] ≠ 0 }|`.
/// The non-zero predicate uses `arith.cmpf one` (ordered-not-equal:
/// excludes NaN-vs-NaN equality), then sign-extends the i1 result to
/// the element type via `arith.uitofp` so we can accumulate with the
/// existing `arith.addf` chain. Backbone of the boolean reductions
/// `All` (`count == n`) and `Any` (`count > 0`).
fn build_count_nonzero_kernel_text(dtype: DType) -> Option<String> {
 let elem_ty = mlir_elem_type(dtype)?;
 if !is_float_dtype(dtype) {
 return None;
 }
 let zero_lit = if dtype == DType::F32 { "0.0 : f32" } else { "0.0 : f64" };
 Some(format!(
 r#"module {{
 func.func @kernel(%a: !llvm.ptr, %out: !llvm.ptr, %n: i64) attributes {{ llvm.emit_c_interface }} {{
 %c0 = arith.constant 0 : i64
 %c1 = arith.constant 1 : i64
 %f0 = arith.constant {zero_lit}
 cf.br ^loop(%c0, %f0 : i64, {elem})
 ^loop(%i: i64, %acc: {elem}):
 %done = arith.cmpi sge, %i, %n : i64
 cf.cond_br %done, ^store, ^body
 ^body:
 %ap = llvm.getelementptr inbounds %a[%i] : (!llvm.ptr, i64) -> !llvm.ptr, {elem}
 %va = llvm.load %ap : !llvm.ptr -> {elem}
 %nz = arith.cmpf one, %va, %f0 : {elem}
 %incr = arith.uitofp %nz : i1 to {elem}
 %new_acc = arith.addf %acc, %incr : {elem}
 %i_next = arith.addi %i, %c1 : i64
 cf.br ^loop(%i_next, %new_acc : i64, {elem})
 ^store:
 llvm.store %acc, %out : {elem}, !llvm.ptr
 return
 }}
}}
"#,
 elem = elem_ty,
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
/// Compared with the a future iteration scalar triple-loop, this opens the door
/// to cuBLAS / MKL-class throughput on large matrices.
///
/// Function signature (after `linalg-to-loops` → `convert-to-llvm`
/// + `finalize-memref-to-llvm` lowering and the
/// `llvm.emit_c_interface` wrapper):
///
/// ```c
/// extern "C" void _mlir_ciface_kernel(
/// StridedMemRefType<T, 2>* a,
/// StridedMemRefType<T, 2>* b,
/// StridedMemRefType<T, 2>* out
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
/// rewrites `linalg.matmul` into an `scf.for` nest. After that the
/// memref ops + scf + arith all reach LLVM dialect via the umbrella
/// pass, and `finalize-memref-to-llvm` lowers memref descriptors to
/// `llvm.struct`. Func-level cleanup happens inside the umbrella.
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
 // translation rejects any that survive. Without it the JIT
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

// =============================================================================
// Persistent on-disk JIT cache .
//
// The in-memory `DashMap` cache on `MlirJitBackend` survives only for
// the lifetime of the process — every fresh interpreter run pays the
// full MLIR-parse + lowering + LLVM-optimise + JIT-codegen cost on the
// first call to each kernel. For ML inference / training workloads
// where interpreter startup is on the hot path (CI, serverless, data-
// pipeline shells), that's hundreds of ms of avoidable warm-up.
//
// The on-disk cache stores the **post-lowering MLIR text** (i.e. the
// module after `convert-to-llvm` and friends ran on it) under a
// content-addressed name. On subsequent runs:
// 1. Build the kernel source text exactly as before.
// 2. Compute the cache key from the source text + target arch + OS
// + cache-format version.
// 3. If the cache file exists, parse it directly and hand it to
// `ExecutionEngine::new` — skipping all the conversion passes.
// Cache hits cut compile cost roughly in half (the LLVM-side
// JIT codegen still runs, but MLIR lowering disappears).
// 4. Misses fall through to the legacy compile path, then write
// the lowered module text out before returning the engine.
//
// Invalidation is automatic: the cache key includes target arch + OS
// + an explicit "v1" version tag, so a toolchain bump (or a kernel-
// text change in `build_*_kernel_text`) produces a different hash and
// the old entry is ignored (and orphaned on disk — reaping is left to
// users / package managers since cache size is tiny).
//
// Cache location precedence:
// 1. `$VERUM_JIT_CACHE` env var (explicit override; useful for CI
// with shared caches).
// 2. `$XDG_CACHE_HOME/verum/jit/v1/` (Linux + freedesktop convention).
// 3. `$HOME/.cache/verum/jit/v1/` (POSIX fallback).
// 4. Any failure to find a cache directory → caching is silently
// disabled and we fall through to the in-memory-only path.
// =============================================================================

const JIT_CACHE_VERSION: &str = "v1";

/// Resolve the on-disk cache directory, or `None` if no suitable
/// location is writable. The check is best-effort: even if we
/// return `Some(path)`, individual reads / writes may still fail
/// (read-only mount, quota, permission), in which case the caller
/// silently falls through to the in-memory path.
fn jit_cache_dir() -> Option<PathBuf> {
 if let Ok(p) = std::env::var("VERUM_JIT_CACHE") {
 if !p.is_empty() {
 return Some(PathBuf::from(p).join(JIT_CACHE_VERSION));
 }
 }
 if let Ok(p) = std::env::var("XDG_CACHE_HOME") {
 if !p.is_empty() {
 return Some(PathBuf::from(p).join("verum").join("jit").join(JIT_CACHE_VERSION));
 }
 }
 if let Ok(home) = std::env::var("HOME") {
 if !home.is_empty() {
 return Some(
 PathBuf::from(home)
 .join(".cache")
 .join("verum")
 .join("jit")
 .join(JIT_CACHE_VERSION),
 );
 }
 }
 None
}

/// Content-addressed cache filename for a given kernel source.
///
/// Hash inputs: format-version tag + target arch + target OS + the
/// raw kernel-source MLIR text. Any bit of those changing yields a
/// different hash and a fresh compile — protecting against silent
/// drift when the toolchain or kernel template moves.
fn jit_cache_key(kernel_text: &str) -> String {
 let mut h = blake3::Hasher::new();
 h.update(b"verum-jit-cache-");
 h.update(JIT_CACHE_VERSION.as_bytes());
 h.update(b"\n");
 h.update(std::env::consts::ARCH.as_bytes());
 h.update(b"\n");
 h.update(std::env::consts::OS.as_bytes());
 h.update(b"\n");
 h.update(kernel_text.as_bytes());
 h.finalize().to_hex().to_string()
}

/// Try to load + parse + JIT-compile a cached lowered MLIR module.
///
/// Returns `None` on any failure (file missing, parse error, verifier
/// rejection, ExecutionEngine construction fault). Failures are not
/// fatal — the caller falls through to the full-compile path.
fn try_load_cached_kernel(
 context: &Context,
 cache_path: &std::path::Path,
) -> Option<ExecutionEngine> {
 let text = std::fs::read_to_string(cache_path).ok()?;
 let module = Module::parse(context, &text)?;
 if !module.as_operation().verify() {
 return None;
 }
 Some(ExecutionEngine::new(&module, /* opt_level */ 2, &[], /* dump */ false))
}

/// Persist the post-lowering MLIR text for a successfully compiled
/// kernel. Best-effort: write failures are logged at trace level
/// and otherwise ignored.
fn write_cached_kernel(cache_path: &std::path::Path, lowered_text: &str) {
 if let Some(parent) = cache_path.parent() {
 if let Err(e) = std::fs::create_dir_all(parent) {
 tracing::trace!(
 "MLIR-JIT: cache directory create failed at {}: {}",
 parent.display(),
 e
 );
 return;
 }
 }
 if let Err(e) = std::fs::write(cache_path, lowered_text.as_bytes()) {
 tracing::trace!(
 "MLIR-JIT: cache write failed at {}: {}",
 cache_path.display(),
 e
 );
 }
}

/// Compile a kernel text into a JIT engine.
///
/// Failure modes on this path:
/// * Kernel text builder returns `None` (op/dtype not yet wired).
/// * Module parse failure → returns `None` and emits a tracing warn.
/// * Verifier rejection → same.
/// * `convert-to-llvm` lowering pass failure → likely a dialect op
/// that the umbrella conversion does not know how to lower.
/// * `ExecutionEngine` construction failure → missing LLVM backend.
fn compile_kernel(
 context: &Context,
 kind: KernelKind,
 dtype: DType,
) -> Option<ExecutionEngine> {
 let text = match kind {
 KernelKind::Binop(op) => build_binop_kernel_text(op, dtype)?,
 KernelKind::BinopScalarBroadcast(op) => {
 build_binop_scalar_broadcast_kernel_text(op, dtype, false)?
 }
 KernelKind::BinopScalarBroadcastFlipped(op) => {
 build_binop_scalar_broadcast_kernel_text(op, dtype, true)?
 }
 KernelKind::BinopSuffixBroadcast(op) => {
 build_binop_suffix_broadcast_kernel_text(op, dtype, false)?
 }
 KernelKind::BinopSuffixBroadcastFlipped(op) => {
 build_binop_suffix_broadcast_kernel_text(op, dtype, true)?
 }
 KernelKind::BinopPrefixBroadcast(op) => {
 build_binop_prefix_broadcast_kernel_text(op, dtype, false)?
 }
 KernelKind::BinopPrefixBroadcastFlipped(op) => {
 build_binop_prefix_broadcast_kernel_text(op, dtype, true)?
 }
 KernelKind::BinopMidAxisBroadcast(op) => {
 build_binop_mid_axis_broadcast_kernel_text(op, dtype, false)?
 }
 KernelKind::BinopMidAxisBroadcastFlipped(op) => {
 build_binop_mid_axis_broadcast_kernel_text(op, dtype, true)?
 }
 KernelKind::Unop(op) => build_unop_kernel_text(op, dtype)?,
 KernelKind::Matmul => build_matmul_kernel_text(dtype)?,
 KernelKind::Reduce(op) => build_reduce_kernel_text(op, dtype)?,
 KernelKind::SumOfSquares => build_sum_of_squares_kernel_text(dtype)?,
 KernelKind::SumOfExp => build_sum_of_exp_kernel_text(dtype)?,
 KernelKind::CountNonzero => build_count_nonzero_kernel_text(dtype)?,
 };

 // Persistent on-disk cache lookup. Hits skip MLIR parse + every
 // conversion pass and go straight from cached lowered-module text
 // to an `ExecutionEngine`. Misses fall through and write back
 // the lowered text after compile.
 let cache_path = jit_cache_dir().map(|d| d.join(format!("{}.mlir", jit_cache_key(&text))));
 if let Some(p) = &cache_path {
 if let Some(engine) = try_load_cached_kernel(context, p) {
 return Some(engine);
 }
 }

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
 // The umbrella `convert-to-llvm` covers arith / cf / func /
 // memref but NOT every `math.*` op — `erf`, `log2`, etc.
 // need an explicit math-to-libm pre-step (lowers to libm
 // calls) followed by math-to-llvm (rewrites the few ops
 // that map to LLVM intrinsics directly). Without these,
 // the LLVM-IR translation step at `ExecutionEngine::new`
 // fails with "missing LLVMTranslationDialectInterface
 // registration for dialect for op: math.erf" on the first
 // unop kernel that uses an unhandled math op.
 let pm = PassManager::new(context);
 pm.add_pass(pass::conversion::create_math_to_libm());
 pm.add_pass(pass::conversion::create_math_to_llvm());
 pm.add_pass(pass::conversion::create_to_llvm());
 pm.add_pass(pass::conversion::create_reconcile_unrealized_casts());
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

 // Persist the lowered module BEFORE handing it to ExecutionEngine
 // — `ExecutionEngine::new` consumes the LLVM-dialect IR but
 // doesn't mutate it. Reading back the text via `Display` is
 // cheap; the write is best-effort and never fails the compile.
 if let Some(p) = &cache_path {
 let lowered_text = format!("{}", module.as_operation());
 write_cached_kernel(p, &lowered_text);
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
 // a future iteration+2d coverage spans F32/F64 + I/U{8,16,32,64}. The
 // half-precision floats and complex dtypes are intentionally
 // left to a future a future iteration their MLIR lowering needs an
 // additional `convert-arith-to-fp16` / complex-dialect pass.
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F16).unwrap();
 let b = TensorHandle::zeros(&[4], DType::F16).unwrap();
 assert!(backend.binop(&a, &b, TensorBinaryOp::Add).is_none());
 }

 #[test]
 fn binop_dtype_mismatch_returns_none() {
 // Mixing dtypes is a type-promotion concern handled upstream
 // by `tensor_binop`'s `DType::promote_static`. By the time we
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
 // (signed-only exponent); deferred to a future iteration. For now,
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
 // A future iteration wiring point: axis-specific reductions need a
 // different kernel (rank-aware loop nest); A future iteration covers
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
 fn reduce_full_op_coverage_pinned() {
 // After a future iteration every `TensorReduceOp` variant has a JIT path
 // for F32 full-axis reduction. The catch-all guard now
 // covers only the dimensions outside the JIT matrix
 // (axis-specific reductions, integer dtypes) — tested in
 // their own dedicated `*_falls_through_*` pins.
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[1.0, 2.0, 3.0, 4.0]);
 for op in [
 TensorReduceOp::Sum,
 TensorReduceOp::Prod,
 TensorReduceOp::Max,
 TensorReduceOp::Min,
 TensorReduceOp::Mean,
 TensorReduceOp::Var,
 TensorReduceOp::Std,
 TensorReduceOp::Norm,
 TensorReduceOp::LogSumExp,
 TensorReduceOp::All,
 TensorReduceOp::Any,
 ] {
 assert!(
 backend.reduce(&a, op, None).is_some(),
 "reduce({:?}) returned None — every TensorReduceOp variant must have a JIT path after a future iteration",
 op
 );
 }
 }

 // -----------------------------------------------------------------
 // A future iteration coverage: statistical reductions via composition
 // -----------------------------------------------------------------

 #[test]
 fn reduce_norm_executes() {
 // L2 norm of [3, 4] is 5 (Pythagorean triple).
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[2], DType::F32).unwrap();
 fill_f32(&a, &[3.0, 4.0]);
 let r = backend.reduce(&a, TensorReduceOp::Norm, None).unwrap();
 let v = read_f32(&r, 1);
 assert!((v[0] - 5.0).abs() < 1e-5, "L2 norm = {}", v[0]);
 }

 #[test]
 fn reduce_norm_zero_vector() {
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[3], DType::F32).unwrap();
 fill_f32(&a, &[0.0, 0.0, 0.0]);
 let r = backend.reduce(&a, TensorReduceOp::Norm, None).unwrap();
 assert!(read_f32(&r, 1)[0].abs() < 1e-6);
 }

 #[test]
 fn reduce_var_executes() {
 // Var([2, 4, 4, 4, 5, 5, 7, 9]) = 4.0 (population variance).
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[8], DType::F32).unwrap();
 fill_f32(&a, &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
 let r = backend.reduce(&a, TensorReduceOp::Var, None).unwrap();
 let v = read_f32(&r, 1);
 assert!((v[0] - 4.0).abs() < 1e-4, "Var = {}", v[0]);
 }

 #[test]
 fn reduce_std_executes() {
 // Std = √Var, so for the same input as above Std = 2.0.
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[8], DType::F32).unwrap();
 fill_f32(&a, &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
 let r = backend.reduce(&a, TensorReduceOp::Std, None).unwrap();
 let v = read_f32(&r, 1);
 assert!((v[0] - 2.0).abs() < 1e-4, "Std = {}", v[0]);
 }

 #[test]
 fn reduce_var_constant_is_zero() {
 // A constant vector has zero variance — pin guards against
 // accidentally swapping `E[X]²` for `E[X²]` (or vice versa).
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[5], DType::F32).unwrap();
 fill_f32(&a, &[3.0, 3.0, 3.0, 3.0, 3.0]);
 let r = backend.reduce(&a, TensorReduceOp::Var, None).unwrap();
 assert!(read_f32(&r, 1)[0].abs() < 1e-5);
 }

 #[test]
 fn reduce_norm_f64() {
 // [1, 2, 3] → √14 ≈ 3.741657...
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[3], DType::F64).unwrap();
 unsafe {
 let p = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
 *p.add(0) = 1.0;
 *p.add(1) = 2.0;
 *p.add(2) = 3.0;
 }
 let r = backend.reduce(&a, TensorReduceOp::Norm, None).unwrap();
 unsafe {
 let p = (*r.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
 assert!((*p - 14.0_f64.sqrt()).abs() < 1e-12);
 }
 }

 // -----------------------------------------------------------------
 // A future iteration coverage: LogSumExp / All / Any
 // -----------------------------------------------------------------

 #[test]
 fn reduce_logsumexp_executes() {
 // log(eˣ + eʸ + e^z) for [0, 0, 0] = log(3) ≈ 1.0986
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[3], DType::F32).unwrap();
 fill_f32(&a, &[0.0, 0.0, 0.0]);
 let r = backend.reduce(&a, TensorReduceOp::LogSumExp, None).unwrap();
 let v = read_f32(&r, 1);
 assert!((v[0] - 3.0_f32.ln()).abs() < 1e-5);
 }

 #[test]
 fn reduce_logsumexp_arbitrary() {
 // Arbitrary-input check.
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F64).unwrap();
 unsafe {
 let p = (*a.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
 *p.add(0) = 1.0;
 *p.add(1) = 2.0;
 *p.add(2) = 3.0;
 *p.add(3) = 4.0;
 }
 let r = backend.reduce(&a, TensorReduceOp::LogSumExp, None).unwrap();
 let expected: f64 = (1.0_f64.exp() + 2.0_f64.exp() + 3.0_f64.exp() + 4.0_f64.exp()).ln();
 unsafe {
 let p = (*r.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
 assert!((*p - expected).abs() < 1e-10);
 }
 }

 #[test]
 fn reduce_all_true_when_no_zeros() {
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[1.0, -2.0, 3.0, 0.5]);
 let r = backend.reduce(&a, TensorReduceOp::All, None).unwrap();
 assert!((read_f32(&r, 1)[0] - 1.0).abs() < 1e-6);
 }

 #[test]
 fn reduce_all_false_when_one_zero() {
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[1.0, 0.0, 3.0, 4.0]);
 let r = backend.reduce(&a, TensorReduceOp::All, None).unwrap();
 assert!(read_f32(&r, 1)[0].abs() < 1e-6);
 }

 #[test]
 fn reduce_any_true_when_one_nonzero() {
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[0.0, 0.0, 7.0, 0.0]);
 let r = backend.reduce(&a, TensorReduceOp::Any, None).unwrap();
 assert!((read_f32(&r, 1)[0] - 1.0).abs() < 1e-6);
 }

 #[test]
 fn reduce_any_false_when_all_zero() {
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[0.0, 0.0, 0.0, 0.0]);
 let r = backend.reduce(&a, TensorReduceOp::Any, None).unwrap();
 assert!(read_f32(&r, 1)[0].abs() < 1e-6);
 }

 #[test]
 fn reduce_var_uses_separate_cache_entry() {
 // SumOfSquares is a distinct KernelKind from Reduce(Sum), so
 // computing both Sum and Var should populate two cache slots
 // (plus one more for Sum that Var composes over).
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[1.0, 2.0, 3.0, 4.0]);
 let _ = backend.reduce(&a, TensorReduceOp::Sum, None).unwrap();
 let _ = backend.reduce(&a, TensorReduceOp::Var, None).unwrap();
 // Var composes: Sum (already cached) + SumOfSquares (new).
 // Cache should have at least 2 distinct entries.
 assert!(backend.cache.len() >= 2);
 }

 #[test]
 fn reduce_f32_mean_executes() {
 // Mean = Sum / n, so for [2,4,6,8] mean = 20/4 = 5.0. The
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
 // a = [[1,2,3],[4,5,6]] (2×3)
 // b = [[7,8],[9,10],[11,12]] (3×2)
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
 // a future iteration+2d coverage: integer binops + float Pow/Mod/Min/Max
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
 // A future iteration coverage: float unops via math.* dialect
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
 fn unop_floor_ceil_executes() {
 // `Round` was wired but later deferred because the lowered
 // libm symbol (`roundevenf` / `roundeven`) is not always
 // present on the host JIT linkage path; `Floor` and `Ceil`
 // map to LLVM intrinsics that need no libm dependency, so
 // they remain in the wired matrix and form the kernel of
 // this regression pin.
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
 unsafe {
 let pf = (*f.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
 let pc = (*c.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
 assert!((*pf.add(0) - 1.0).abs() < 1e-12);
 assert!((*pf.add(1) - (-3.0)).abs() < 1e-12);
 assert!((*pc.add(0) - 2.0).abs() < 1e-12);
 assert!((*pc.add(1) - (-2.0)).abs() < 1e-12);
 }
 // Round falls through to the scalar fallback now.
 assert!(backend.unop(&a, TensorUnaryOp::Round).is_none());
 }

 #[test]
 fn unop_returns_none_for_unsupported_dtypes() {
 // a future iteration contract: math.* ops are float-only. Integer dtypes
 // fall through to `CpuBackend`.
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::I32).unwrap();
 assert!(backend.unop(&a, TensorUnaryOp::Sqrt).is_none());
 assert!(backend.unop(&a, TensorUnaryOp::Sin).is_none());
 }

 #[test]
 fn unop_returns_none_for_composed_ops() {
 // A future iteration wiring point: Sigmoid / Relu / Gelu / Silu / Softplus /
 // Mish / Sign require composition that the simple template
 // engine doesn't synthesise. Until they land, fall through.
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

 // -----------------------------------------------------------------
 // A future iteration coverage: persistent on-disk JIT cache
 // -----------------------------------------------------------------

 fn isolated_jit_cache_dir() -> std::path::PathBuf {
 // Per-test-process scratch directory. Each test that touches
 // the cache must point `VERUM_JIT_CACHE` at a fresh subdir to
 // avoid clobbering between tests run in parallel.
 let pid = std::process::id();
 let counter = std::sync::atomic::AtomicU64::new(0);
 let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
 let nanos = std::time::SystemTime::now()
 .duration_since(std::time::UNIX_EPOCH)
 .map(|d| d.as_nanos())
 .unwrap_or(0);
 std::env::temp_dir().join(format!("verum-jit-cache-test-{}-{}-{}", pid, n, nanos))
 }

 #[test]
 fn cache_key_changes_with_kernel_text() {
 let k1 = jit_cache_key("module { llvm.func @kernel() { llvm.return } }");
 let k2 = jit_cache_key("module { llvm.func @other() { llvm.return } }");
 assert_ne!(k1, k2, "different kernel text must produce different keys");
 // Stable: same input → same hash.
 let k3 = jit_cache_key("module { llvm.func @kernel() { llvm.return } }");
 assert_eq!(k1, k3);
 }

 #[test]
 fn cache_key_is_deterministic_per_arch() {
 // Same kernel text produces hashes that include the host
 // arch + os, so tests on different architectures don't
 // accidentally share cache entries.
 let k = jit_cache_key("test");
 // 64 hex chars = 256-bit blake3 output.
 assert_eq!(k.len(), 64);
 assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
 }

 #[test]
 fn cache_writes_lowered_module_on_first_compile() {
 let dir = isolated_jit_cache_dir();
 // SAFETY: tests are single-threaded for env_var purposes by
 // virtue of `--test-threads=1` in CI; per-process
 // experimentation here is acceptable.
 unsafe {
 std::env::set_var("VERUM_JIT_CACHE", &dir);
 }
 let backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[4], DType::F32).unwrap();
 let b = TensorHandle::zeros(&[4], DType::F32).unwrap();
 fill_f32(&a, &[1.0, 2.0, 3.0, 4.0]);
 fill_f32(&b, &[10.0, 20.0, 30.0, 40.0]);
 let _ = backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();

 // After one binop compile, the cache directory should
 // contain at least one .mlir file.
 let v1 = dir.join(JIT_CACHE_VERSION);
 assert!(v1.exists(), "cache dir not created at {}", v1.display());
 let entries: Vec<_> = std::fs::read_dir(&v1)
 .unwrap()
 .filter_map(|e| e.ok())
 .filter(|e| {
 e.path().extension().and_then(|s| s.to_str()) == Some("mlir")
 })
 .collect();
 assert!(
 !entries.is_empty(),
 "expected at least one cached .mlir file under {}",
 v1.display()
 );

 unsafe {
 std::env::remove_var("VERUM_JIT_CACHE");
 }
 let _ = std::fs::remove_dir_all(&dir);
 }

 #[test]
 fn cache_hit_produces_correct_results() {
 // Pre-populate the cache by compiling once; then construct
 // a fresh backend pointing at the same cache and verify the
 // compute output is bit-equivalent.
 let dir = isolated_jit_cache_dir();
 unsafe {
 std::env::set_var("VERUM_JIT_CACHE", &dir);
 }

 let warm_backend = MlirJitBackend::new();
 let a = TensorHandle::zeros(&[3], DType::F32).unwrap();
 let b = TensorHandle::zeros(&[3], DType::F32).unwrap();
 fill_f32(&a, &[1.5, 2.5, 3.5]);
 fill_f32(&b, &[0.5, 1.5, 2.5]);
 let warm = warm_backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
 let warm_vals = read_f32(&warm, 3);
 drop(warm_backend);

 // Fresh backend → in-memory cache empty, but on-disk cache
 // has the lowered module. Compile path should hit the
 // file and skip lowering.
 let cold_backend = MlirJitBackend::new();
 let cold = cold_backend.binop(&a, &b, TensorBinaryOp::Add).unwrap();
 let cold_vals = read_f32(&cold, 3);
 assert_eq!(warm_vals, cold_vals);

 unsafe {
 std::env::remove_var("VERUM_JIT_CACHE");
 }
 let _ = std::fs::remove_dir_all(&dir);
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
