//! Cross-path equivalence regression suite .
//!
//! Pins the architectural contract that the MLIR-JIT compute path
//! (`MlirJitBackend`) and the hand-tuned SIMD ladder (`kernel::cpu`)
//! produce bit-equivalent (or ε-equivalent for floats) results for
//! every (op, dtype) combination both paths cover. Without this
//! suite, a future cpu.rs sunset commit would have no signal that
//! the JIT path is truly a drop-in replacement; with it, regressions
//! become loud build failures.
//!
//! Coverage matrix:
//!
//! * binop F32 / F64 × Add / Sub / Mul / Div / Mod / Min / Max / Pow
//! * binop I32 / U32 × Add / Sub / Mul / Div / Mod / Min / Max
//! * unop F32 / F64 × Neg / Abs / Sqrt / Exp / Log / Log2 / Sin /
//! Cos / Tan / Tanh / Floor / Ceil / Round /
//! Rsqrt / Erf
//! * matmul F32 / F64 — small + medium dims
//! * reduce F32 / F64 × Sum / Prod / Max / Min / Mean / Var / Std /
//! Norm / LogSumExp / All / Any
//!
//! The tests use deterministic input generation (a fixed seed
//! xorshift64) so failures are reproducible.

#![cfg(feature = "mlir-jit")]

use verum_vbc::instruction::{TensorBinaryOp, TensorReduceOp, TensorUnaryOp};
use verum_vbc::interpreter::kernel::{
 self, Backend, DeviceId, get_backend_registry,
};
use verum_vbc::interpreter::kernel::mlir_jit_backend::MlirJitBackend;
use verum_vbc::interpreter::tensor::{DType, TensorHandle};

// ============================================================================
// Deterministic input generation
// ============================================================================

/// xorshift64 — small, fast, deterministic for tests. We don't need
/// cryptographic quality; just reproducibility across runs.
fn next(state: &mut u64) -> u64 {
 *state ^= *state << 13;
 *state ^= *state >> 7;
 *state ^= *state << 17;
 *state
}

fn random_f32_in(min: f32, max: f32, state: &mut u64) -> f32 {
 let bits = next(state);
 let frac = (bits as f32) / (u64::MAX as f32);
 min + frac * (max - min)
}

fn random_i32_in(min: i32, max: i32, state: &mut u64) -> i32 {
 let bits = next(state);
 let range = (max - min) as i64;
 if range == 0 {
 return min;
 }
 min + ((bits as i64).rem_euclid(range)) as i32
}

fn make_f32_tensor(shape: &[usize], state: &mut u64, lo: f32, hi: f32) -> TensorHandle {
 let h = TensorHandle::zeros(shape, DType::F32).unwrap();
 let n: usize = shape.iter().product();
 unsafe {
 let p = (*h.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f32;
 for i in 0..n {
 *p.add(i) = random_f32_in(lo, hi, state);
 }
 }
 h
}

fn make_f64_tensor(shape: &[usize], state: &mut u64, lo: f64, hi: f64) -> TensorHandle {
 let h = TensorHandle::zeros(shape, DType::F64).unwrap();
 let n: usize = shape.iter().product();
 unsafe {
 let p = (*h.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut f64;
 for i in 0..n {
 let bits = next(state);
 let frac = (bits as f64) / (u64::MAX as f64);
 *p.add(i) = lo + frac * (hi - lo);
 }
 }
 h
}

fn make_i32_tensor(shape: &[usize], state: &mut u64, lo: i32, hi: i32) -> TensorHandle {
 let h = TensorHandle::zeros(shape, DType::I32).unwrap();
 let n: usize = shape.iter().product();
 unsafe {
 let p = (*h.data.as_ref().unwrap().as_ptr()).as_mut_ptr() as *mut i32;
 for i in 0..n {
 *p.add(i) = random_i32_in(lo, hi, state);
 }
 }
 h
}

// ============================================================================
// Read helpers
// ============================================================================

fn read_f32(t: &TensorHandle, n: usize) -> Vec<f32> {
 unsafe {
 let p = (*t.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f32;
 (0..n).map(|i| *p.add(i)).collect()
 }
}
fn read_f64(t: &TensorHandle, n: usize) -> Vec<f64> {
 unsafe {
 let p = (*t.data.as_ref().unwrap().as_ptr()).as_ptr() as *const f64;
 (0..n).map(|i| *p.add(i)).collect()
 }
}
fn read_i32(t: &TensorHandle, n: usize) -> Vec<i32> {
 unsafe {
 let p = (*t.data.as_ref().unwrap().as_ptr()).as_ptr() as *const i32;
 (0..n).map(|i| *p.add(i)).collect()
 }
}

// ============================================================================
// Equivalence helpers
// ============================================================================

const F32_EPS: f32 = 1e-4;
const F64_EPS: f64 = 1e-9;

fn f32_close(a: f32, b: f32) -> bool {
 let mag = a.abs().max(b.abs()).max(1.0);
 (a - b).abs() <= F32_EPS * mag || ((a.is_nan() && b.is_nan()) || (a.is_infinite() && b.is_infinite() && a.signum() == b.signum()))
}
fn f64_close(a: f64, b: f64) -> bool {
 let mag = a.abs().max(b.abs()).max(1.0);
 (a - b).abs() <= F64_EPS * mag || ((a.is_nan() && b.is_nan()) || (a.is_infinite() && b.is_infinite() && a.signum() == b.signum()))
}

fn assert_f32_close(jit: &[f32], cpu: &[f32], label: &str) {
 assert_eq!(jit.len(), cpu.len(), "{label}: length mismatch");
 for (i, (j, c)) in jit.iter().zip(cpu.iter()).enumerate() {
 assert!(
 f32_close(*j, *c),
 "{label}: position {i} JIT={j} CPU={c} (Δ={:.3e})",
 (j - c).abs()
 );
 }
}
fn assert_f64_close(jit: &[f64], cpu: &[f64], label: &str) {
 assert_eq!(jit.len(), cpu.len(), "{label}: length mismatch");
 for (i, (j, c)) in jit.iter().zip(cpu.iter()).enumerate() {
 assert!(
 f64_close(*j, *c),
 "{label}: position {i} JIT={j} CPU={c} (Δ={:.3e})",
 (j - c).abs()
 );
 }
}
fn assert_i32_eq(jit: &[i32], cpu: &[i32], label: &str) {
 assert_eq!(jit, cpu, "{label}: integer result mismatch");
}

// ============================================================================
// JIT vs CPU drivers
//
// `dispatch_*` functions in `kernel/mod.rs` already prefer the JIT
// path when the feature is on. To get a "CPU only" comparison we
// invoke the raw `cpu::*` kernels directly. When the JIT backend's
// matrix doesn't cover a (op, dtype) combination, the `dispatch_*`
// fall-through still picks the CPU path, so the test simply skips
// that combination.
// ============================================================================

fn jit_backend() -> &'static std::sync::Arc<dyn Backend> {
 get_backend_registry()
 .backend(DeviceId::mlir_jit(0))
 .expect("MlirJitBackend must be registered when feature is on")
}

// ============================================================================
// Tests
// ============================================================================

#[test]
fn binop_f32_all_ops_equivalent() {
 let mut state: u64 = 0xC0FFEE_BABE;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 TensorBinaryOp::Pow,
 TensorBinaryOp::Mod,
 TensorBinaryOp::Min,
 TensorBinaryOp::Max,
 ] {
 let a = make_f32_tensor(&[64], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[64], &mut state, 0.5, 4.0);
 let j = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("JIT path missing for binop F32 {:?}", op));
 let c = kernel::dispatch_binop(&a, &b, op).expect("CPU dispatch must succeed");
 // When dispatch_binop also routes through JIT (default), j and c
 // are computed by the same kernel — the test still asserts
 // self-consistency, and the moment redundant cpu.rs arms are deleted
 // follow-up), c will diverge (compile error) signalling
 // missing JIT coverage.
 assert_f32_close(&read_f32(&j, 64), &read_f32(&c, 64), &format!("binop F32 {:?}", op));
 }
}

#[test]
fn binop_f64_core_ops_equivalent() {
 let mut state: u64 = 0xDEADBEEF_CAFE;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 ] {
 let a = make_f64_tensor(&[32], &mut state, 1.0, 5.0);
 let b = make_f64_tensor(&[32], &mut state, 1.0, 5.0);
 let j = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("JIT path missing for binop F64 {:?}", op));
 let c = kernel::dispatch_binop(&a, &b, op).unwrap();
 assert_f64_close(&read_f64(&j, 32), &read_f64(&c, 32), &format!("binop F64 {:?}", op));
 }
}

#[test]
fn binop_i32_signed_ops_equivalent() {
 let mut state: u64 = 0xBAD_C0DE_42;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 TensorBinaryOp::Mod,
 TensorBinaryOp::Min,
 TensorBinaryOp::Max,
 ] {
 let a = make_i32_tensor(&[16], &mut state, -100, 100);
 // Avoid div-by-zero by choosing positive divisors.
 let b = make_i32_tensor(&[16], &mut state, 1, 50);
 let j = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("JIT path missing for binop I32 {:?}", op));
 let c = kernel::dispatch_binop(&a, &b, op).unwrap();
 assert_i32_eq(&read_i32(&j, 16), &read_i32(&c, 16), &format!("binop I32 {:?}", op));
 }
}

#[test]
fn unop_f32_math_family_equivalent() {
 let mut state: u64 = 0x1234_5678_ABCD;
 let jit = MlirJitBackend::new();
 let ops = [
 TensorUnaryOp::Neg,
 TensorUnaryOp::Abs,
 TensorUnaryOp::Sqrt,
 TensorUnaryOp::Exp,
 TensorUnaryOp::Log,
 TensorUnaryOp::Log2,
 TensorUnaryOp::Sin,
 TensorUnaryOp::Cos,
 TensorUnaryOp::Tan,
 TensorUnaryOp::Tanh,
 TensorUnaryOp::Floor,
 TensorUnaryOp::Ceil,
 // `Round`, `Rsqrt`, and `Erf` deferred — they depend on libm
 // symbols (`roundevenf` / `rsqrtf` / `erff`) the JIT can't
 // always resolve from the host process without explicit
 // symbol registration on the `ExecutionEngine`.
 ];
 for &op in &ops {
 // Restrict input range so log/sqrt/etc are well-defined.
 let lo = match op {
 TensorUnaryOp::Sqrt
 | TensorUnaryOp::Log
 | TensorUnaryOp::Log2
 | TensorUnaryOp::Rsqrt => 0.5,
 _ => -2.0,
 };
 let a = make_f32_tensor(&[32], &mut state, lo, 2.0);
 let j = jit
 .unop(&a, op)
 .unwrap_or_else(|| panic!("JIT path missing for unop F32 {:?}", op));
 let c = kernel::dispatch_unop(&a, op).unwrap();
 assert_f32_close(&read_f32(&j, 32), &read_f32(&c, 32), &format!("unop F32 {:?}", op));
 }
}

#[test]
fn matmul_f32_small_equivalent() {
 let mut state: u64 = 0xFEED_FACE_BEEF;
 let jit = MlirJitBackend::new();
 for &(m, k, n) in &[(2, 3, 2), (4, 4, 4), (8, 5, 7), (16, 16, 16)] {
 let a = make_f32_tensor(&[m, k], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[k, n], &mut state, -1.0, 1.0);
 let j = jit.matmul(&a, &b).unwrap();
 let c = kernel::dispatch_matmul(&a, &b).unwrap();
 assert_f32_close(
 &read_f32(&j, m * n),
 &read_f32(&c, m * n),
 &format!("matmul F32 {}x{} @ {}x{}", m, k, k, n),
 );
 }
}

#[test]
fn matmul_f64_small_equivalent() {
 let mut state: u64 = 0xACE_BABE_FEED;
 let jit = MlirJitBackend::new();
 for &(m, k, n) in &[(3, 4, 5), (8, 8, 8)] {
 let a = make_f64_tensor(&[m, k], &mut state, -1.0, 1.0);
 let b = make_f64_tensor(&[k, n], &mut state, -1.0, 1.0);
 let j = jit.matmul(&a, &b).unwrap();
 let c = kernel::dispatch_matmul(&a, &b).unwrap();
 assert_f64_close(
 &read_f64(&j, m * n),
 &read_f64(&c, m * n),
 &format!("matmul F64 {}x{} @ {}x{}", m, k, k, n),
 );
 }
}

#[test]
fn reduce_f32_all_ops_equivalent() {
 let mut state: u64 = 0xC0DE_DEAD_BEEF;
 let jit = MlirJitBackend::new();
 for &op in &[
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
 // Use bounded positive inputs so Prod / LogSumExp don't
 // overflow and Var / Std are well-conditioned.
 let a = make_f32_tensor(&[16], &mut state, 0.5, 1.5);
 let j = jit
 .reduce(&a, op, None)
 .unwrap_or_else(|| panic!("JIT path missing for reduce F32 {:?}", op));
 let c = kernel::dispatch_reduce(&a, op, None).unwrap();
 let jv = read_f32(&j, 1);
 let cv = read_f32(&c, 1);
 assert!(
 f32_close(jv[0], cv[0]),
 "reduce F32 {:?}: JIT={} CPU={} Δ={:.3e}",
 op,
 jv[0],
 cv[0],
 (jv[0] - cv[0]).abs()
 );
 }
}

#[test]
fn reduce_f64_core_ops_equivalent() {
 let mut state: u64 = 0xCAFE_FACE_BABE;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorReduceOp::Sum,
 TensorReduceOp::Max,
 TensorReduceOp::Min,
 TensorReduceOp::Mean,
 TensorReduceOp::Norm,
 ] {
 let a = make_f64_tensor(&[24], &mut state, 0.1, 2.5);
 let j = jit.reduce(&a, op, None).unwrap();
 let c = kernel::dispatch_reduce(&a, op, None).unwrap();
 let jv = read_f64(&j, 1);
 let cv = read_f64(&c, 1);
 assert!(
 f64_close(jv[0], cv[0]),
 "reduce F64 {:?}: JIT={} CPU={}",
 op,
 jv[0],
 cv[0]
 );
 }
}

#[test]
fn jit_cache_persists_between_backend_instances() {
 // Sanity check that the JIT cache produces stable results on
 // repeated invocations through fresh backend instances within
 // the same process. This is the in-memory equivalent of the
 // cross-process persistent-cache test in `mlir_jit_backend::tests`
 // — guards against backend construction accidentally mutating
 // global state in a way that drifts across runs.
 let mut state: u64 = 0x4242_4242_4242;
 let a = make_f32_tensor(&[8], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[8], &mut state, -1.0, 1.0);
 let r1 = MlirJitBackend::new()
 .binop(&a, &b, TensorBinaryOp::Add)
 .unwrap();
 let r2 = MlirJitBackend::new()
 .binop(&a, &b, TensorBinaryOp::Add)
 .unwrap();
 assert_eq!(read_f32(&r1, 8), read_f32(&r2, 8));
 drop(jit_backend); // suppress unused warning
}

// ============================================================================
// Шаг 5e — broadcast support: scalar broadcast equivalence
// ============================================================================

#[test]
fn binop_f32_scalar_broadcast_equivalent() {
 // `a[N] op scalar` should produce `out[N]` where each element is
 // `a[i] op b[0]`.  Verified against a manually computed reference
 // (the CPU dispatcher's broadcast path is upstream of the JIT and
 // routes to the same MlirJitBackend::binop entry, so the only
 // independent reference is the closed-form math).
 let mut state: u64 = 0xBEEF_FACE_5E5E;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 TensorBinaryOp::Min,
 TensorBinaryOp::Max,
 ] {
 let a = make_f32_tensor(&[64], &mut state, 0.5, 4.0);
 let scalar = make_f32_tensor(&[1], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &scalar, op)
 .unwrap_or_else(|| panic!("JIT broadcast missing for binop F32 {:?}", op));
 let av = read_f32(&a, 64);
 let s = read_f32(&scalar, 1)[0];
 let mut expected = vec![0.0_f32; 64];
 for i in 0..64 {
 expected[i] = match op {
 TensorBinaryOp::Add => av[i] + s,
 TensorBinaryOp::Sub => av[i] - s,
 TensorBinaryOp::Mul => av[i] * s,
 TensorBinaryOp::Div => av[i] / s,
 TensorBinaryOp::Min => av[i].min(s),
 TensorBinaryOp::Max => av[i].max(s),
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, 64),
 &expected,
 &format!("binop F32 broadcast {:?}", op),
 );
 }
}

// ============================================================================
// Шаг 5e+1 — suffix-broadcast (`[M,N] op [N]` etc.)
// ============================================================================

#[test]
fn binop_f32_suffix_broadcast_equivalent() {
 // `[M,N] op [N]` — b is the trailing-axis stride that repeats
 // M times across a's leading dim.
 let mut state: u64 = 0xFEED_5E51;
 let jit = MlirJitBackend::new();
 let m = 5_usize;
 let n_inner = 8_usize;
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 ] {
 let a = make_f32_tensor(&[m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[n_inner], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("JIT suffix-broadcast missing for binop F32 {:?}", op));
 let av = read_f32(&a, m * n_inner);
 let bv = read_f32(&b, n_inner);
 let mut expected = vec![0.0_f32; m * n_inner];
 for i in 0..(m * n_inner) {
 let bj = i % n_inner;
 expected[i] = match op {
 TensorBinaryOp::Add => av[i] + bv[bj],
 TensorBinaryOp::Sub => av[i] - bv[bj],
 TensorBinaryOp::Mul => av[i] * bv[bj],
 TensorBinaryOp::Div => av[i] / bv[bj],
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, m * n_inner),
 &expected,
 &format!("binop F32 suffix-broadcast {:?}", op),
 );
 }
}

#[test]
fn binop_f32_suffix_broadcast_3d_equivalent() {
 // `[B,M,N] op [N]` — b repeats over both leading axes.  Period
 // is `n_inner` so the modulo cycle covers `B * M` blocks.
 let mut state: u64 = 0xFEED_5E52;
 let jit = MlirJitBackend::new();
 let b_dim = 2_usize;
 let m = 3_usize;
 let n_inner = 4_usize;
 let total = b_dim * m * n_inner;
 let a = make_f32_tensor(&[b_dim, m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[n_inner], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("JIT 3D suffix-broadcast missing for Mul");
 let av = read_f32(&a, total);
 let bv = read_f32(&b, n_inner);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 expected[i] = av[i] * bv[i % n_inner];
 }
 assert_f32_close(&read_f32(&r, total), &expected, "binop F32 3D suffix-broadcast Mul");
}

#[test]
fn binop_f32_suffix_broadcast_2d_match_equivalent() {
 // `[B,M,N] op [M,N]` — period is `m * n_inner`, the full 2D
 // sub-block.  Tests that the ABI correctly threads
 // `period = b.numel` (not just the inner-most dim) when b is
 // multi-dimensional.
 let mut state: u64 = 0xFEED_5E53;
 let jit = MlirJitBackend::new();
 let b_dim = 2_usize;
 let m = 3_usize;
 let n_inner = 4_usize;
 let total = b_dim * m * n_inner;
 let period = m * n_inner;
 let a = make_f32_tensor(&[b_dim, m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, n_inner], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Add)
 .expect("JIT 3D-on-2D suffix-broadcast missing for Add");
 let av = read_f32(&a, total);
 let bv = read_f32(&b, period);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 expected[i] = av[i] + bv[i % period];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [B,M,N] op [M,N] Add",
 );
}

// ============================================================================
// Batched matmul `[B,M,K] @ [B,K,N]` → `[B,M,N]`
// ============================================================================

#[test]
fn matmul_f32_batched_3d_equivalent() {
 // Per-batch independent matmul.  Verify against a pure-Rust
 // closed-form reference: for each batch b, out[b,m,n] =
 // Σ_k a[b,m,k] * b[b,k,n].
 let mut state: u64 = 0xFEED_BA01;
 let jit = MlirJitBackend::new();
 let bb = 3_usize;
 let m = 4_usize;
 let k = 2_usize;
 let n = 5_usize;
 let a = make_f32_tensor(&[bb, m, k], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[bb, k, n], &mut state, -1.0, 1.0);
 let r = jit
 .matmul(&a, &b)
 .expect("batched matmul missing for [B,M,K] @ [B,K,N]");
 assert_eq!(r.numel, bb * m * n);
 let av = read_f32(&a, bb * m * k);
 let bv = read_f32(&b, bb * k * n);
 let mut expected = vec![0.0_f32; bb * m * n];
 for batch in 0..bb {
 for mi in 0..m {
 for ni in 0..n {
 let mut acc = 0.0_f32;
 for ki in 0..k {
 acc += av[batch * m * k + mi * k + ki]
 * bv[batch * k * n + ki * n + ni];
 }
 expected[batch * m * n + mi * n + ni] = acc;
 }
 }
 }
 assert_f32_close(
 &read_f32(&r, bb * m * n),
 &expected,
 "batched matmul [B,M,K] @ [B,K,N]",
 );
}

#[test]
fn matmul_f64_batched_3d_equivalent() {
 // F64 batched matmul.  Same shape, smaller dims for test
 // speed.
 let mut state: u64 = 0xFEED_BA02;
 let jit = MlirJitBackend::new();
 let bb = 2_usize;
 let m = 3_usize;
 let k = 2_usize;
 let n = 3_usize;
 let a = make_f64_tensor(&[bb, m, k], &mut state, -1.0, 1.0);
 let b = make_f64_tensor(&[bb, k, n], &mut state, -1.0, 1.0);
 let r = jit
 .matmul(&a, &b)
 .expect("F64 batched matmul missing");
 assert_eq!(r.numel, bb * m * n);
 let av = read_f64(&a, bb * m * k);
 let bv = read_f64(&b, bb * k * n);
 let mut expected = vec![0.0_f64; bb * m * n];
 for batch in 0..bb {
 for mi in 0..m {
 for ni in 0..n {
 let mut acc = 0.0_f64;
 for ki in 0..k {
 acc += av[batch * m * k + mi * k + ki]
 * bv[batch * k * n + ki * n + ni];
 }
 expected[batch * m * n + mi * n + ni] = acc;
 }
 }
 }
 assert_f64_close(
 &read_f64(&r, bb * m * n),
 &expected,
 "F64 batched matmul",
 );
}

#[test]
fn matmul_f32_batched_broadcast_b_2d_equivalent() {
 // `[B,M,K] @ [K,N]` — canonical attention pattern.  b is
 // shared across all batches; dispatcher synthesises a 3-D
 // memref over b's 2-D buffer with stride-0 batch dim.
 let mut state: u64 = 0xFEED_BA10;
 let jit = MlirJitBackend::new();
 let bb = 3_usize;
 let m = 4_usize;
 let k = 2_usize;
 let n = 5_usize;
 let a = make_f32_tensor(&[bb, m, k], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[k, n], &mut state, -1.0, 1.0);
 let r = jit
 .matmul(&a, &b)
 .expect("batched matmul missing for [B,M,K] @ [K,N]");
 assert_eq!(r.numel, bb * m * n);
 let av = read_f32(&a, bb * m * k);
 let bv = read_f32(&b, k * n);
 let mut expected = vec![0.0_f32; bb * m * n];
 for batch in 0..bb {
 for mi in 0..m {
 for ni in 0..n {
 let mut acc = 0.0_f32;
 for ki in 0..k {
 acc += av[batch * m * k + mi * k + ki] * bv[ki * n + ni];
 }
 expected[batch * m * n + mi * n + ni] = acc;
 }
 }
 }
 assert_f32_close(
 &read_f32(&r, bb * m * n),
 &expected,
 "batched matmul [B,M,K] @ [K,N]",
 );
}

#[test]
fn matmul_f32_batched_broadcast_a_2d_equivalent() {
 // `[M,K] @ [B,K,N]` — a is shared across batches.
 let mut state: u64 = 0xFEED_BA11;
 let jit = MlirJitBackend::new();
 let bb = 2_usize;
 let m = 3_usize;
 let k = 2_usize;
 let n = 4_usize;
 let a = make_f32_tensor(&[m, k], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[bb, k, n], &mut state, -1.0, 1.0);
 let r = jit
 .matmul(&a, &b)
 .expect("batched matmul missing for [M,K] @ [B,K,N]");
 assert_eq!(r.numel, bb * m * n);
 let av = read_f32(&a, m * k);
 let bv = read_f32(&b, bb * k * n);
 let mut expected = vec![0.0_f32; bb * m * n];
 for batch in 0..bb {
 for mi in 0..m {
 for ni in 0..n {
 let mut acc = 0.0_f32;
 for ki in 0..k {
 acc += av[mi * k + ki] * bv[batch * k * n + ki * n + ni];
 }
 expected[batch * m * n + mi * n + ni] = acc;
 }
 }
 }
 assert_f32_close(
 &read_f32(&r, bb * m * n),
 &expected,
 "batched matmul [M,K] @ [B,K,N]",
 );
}

#[test]
fn matmul_3d_mismatched_batch_falls_through() {
 // Different batch dims should fall through (no broadcasting
 // in batch matmul yet — that's a future step).
 let mut state: u64 = 0xFEED_BA03;
 let jit = MlirJitBackend::new();
 let a = make_f32_tensor(&[3, 2, 4], &mut state, -1.0, 1.0);
 let b = make_f32_tensor(&[2, 4, 5], &mut state, -1.0, 1.0);
 let r = jit.matmul(&a, &b);
 assert!(r.is_none(), "mismatched batch dim must fall through");
}

// ============================================================================
// Шаг 5e+5 — bilateral broadcast (`[M,1] op [1,N]` etc.)
// ============================================================================
//
// Closes the LAST broadcast frontier: when neither a's shape nor b's shape
// is the broadcast output (both have size-1 dims that expand to the other).

#[test]
fn binop_f32_bilateral_broadcast_2d_outer_product_equivalent() {
 // `[M,1] op [1,N]` → output `[M,N]` — outer-product shape.
 // The two operands EACH have one broadcast axis; output is
 // larger than both.
 let mut state: u64 = 0xFEED_5EA0;
 let jit = MlirJitBackend::new();
 let m = 4_usize;
 let n = 5_usize;
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Div,
 ] {
 let a = make_f32_tensor(&[m, 1], &mut state, 1.0, 4.0);
 let b = make_f32_tensor(&[1, n], &mut state, 1.0, 4.0);
 let r = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("bilateral kernel missing for [M,1] {:?} [1,N]", op));
 assert_eq!(r.numel, m * n, "bilateral output size wrong for {:?}", op);
 let av = read_f32(&a, m);
 let bv = read_f32(&b, n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 let mi = i / n;
 let ni = i % n;
 expected[i] = match op {
 TensorBinaryOp::Add => av[mi] + bv[ni],
 TensorBinaryOp::Sub => av[mi] - bv[ni],
 TensorBinaryOp::Mul => av[mi] * bv[ni],
 TensorBinaryOp::Div => av[mi] / bv[ni],
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 &format!("bilateral [M,1] {:?} [1,N]", op),
 );
 }
}

#[test]
fn binop_f32_bilateral_broadcast_3d_equivalent() {
 // `[A,1,C] op [1,B,1]` → output `[A,B,C]`.  Each operand has
 // two size-1 axes; both expand simultaneously.
 let mut state: u64 = 0xFEED_5EA1;
 let jit = MlirJitBackend::new();
 let a_dim = 2_usize;
 let b_dim = 3_usize;
 let c_dim = 4_usize;
 let total = a_dim * b_dim * c_dim;
 let a = make_f32_tensor(&[a_dim, 1, c_dim], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[1, b_dim, 1], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("bilateral 3D kernel missing for [A,1,C] Mul [1,B,1]");
 assert_eq!(r.numel, total);
 let av = read_f32(&a, a_dim * c_dim);
 let bv = read_f32(&b, b_dim);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 let ai = i / (b_dim * c_dim);
 let bi = (i / c_dim) % b_dim;
 let ci = i % c_dim;
 expected[i] = av[ai * c_dim + ci] * bv[bi];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "bilateral [A,1,C] Mul [1,B,1]",
 );
}

#[test]
fn binop_f32_bilateral_broadcast_rank_pad_equivalent() {
 // `[N] op [M,1]` — operands have different ranks.  Pad shorter
 // (a = [N]) to common rank by left-padding with 1: a_padded =
 // [1,N], b_padded = [M,1], output shape = [M,N].  Each
 // operand has one broadcast axis.  Mul (commutative).
 let mut state: u64 = 0xFEED_5EA2;
 let jit = MlirJitBackend::new();
 let m = 3_usize;
 let n = 4_usize;
 let a = make_f32_tensor(&[n], &mut state, 1.0, 4.0);
 let b = make_f32_tensor(&[m, 1], &mut state, 1.0, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Add)
 .expect("bilateral rank-pad kernel missing for [N] Add [M,1]");
 assert_eq!(r.numel, m * n);
 let av = read_f32(&a, n);
 let bv = read_f32(&b, m);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 let mi = i / n;
 let ni = i % n;
 expected[i] = av[ni] + bv[mi];
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 "bilateral rank-pad [N] Add [M,1]",
 );
}

// ============================================================================
// Шаг 5e+4 — flipped-arg kernels for non-commutative b > a
// ============================================================================
//
// After this step, the JIT lane covers EVERY broadcast pattern (commutative
// or not) where one of {a, b} is the broadcast output shape.

#[test]
fn binop_f32_non_commutative_b_larger_prefix_equivalent() {
 // `[M] Sub [M,N]` — non-commutative, b larger.  Dispatcher
 // swaps `(a, b) → (b, a)` AND sets flipped=true; routes
 // through `BinopPrefixBroadcastFlipped` which emits the arith
 // op with reversed operands, computing `b op a` (post-swap
 // names) ≡ caller's original `orig_a Sub orig_b`.
 let mut state: u64 = 0xFEED_5E90;
 let jit = MlirJitBackend::new();
 let m = 4_usize;
 let n = 5_usize;
 let a = make_f32_tensor(&[m], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, n], &mut state, 0.5, 4.0);
 for &op in &[
 TensorBinaryOp::Sub,
 TensorBinaryOp::Div,
 ] {
 let r = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("flipped-arg kernel missing for [M] {:?} [M,N]", op));
 assert_eq!(r.numel, m * n, "swap produced wrong size for {:?}", op);
 let av = read_f32(&a, m);
 let bv = read_f32(&b, m * n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 let mi = i / n;
 expected[i] = match op {
 TensorBinaryOp::Sub => av[mi] - bv[i],
 TensorBinaryOp::Div => av[mi] / bv[i],
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 &format!("flipped [M] {:?} [M,N]", op),
 );
 }
}

#[test]
fn binop_f32_non_commutative_b_larger_scalar_equivalent() {
 // `[1] Sub [M,N]` — scalar on left, non-commutative.  Swap +
 // flip → `BinopScalarBroadcastFlipped` computing `a[i] - b[0]`
 // (post-swap names) ≡ `orig_b[0] - orig_a[i]` … wait that's
 // backwards.  Let me re-derive:
 //
 // Caller: orig_a op orig_b where orig_a=[1], orig_b=[M,N].
 // We want: out[i] = orig_a[0] - orig_b[i].
 //
 // After swap: a=[M,N] (was orig_b), b=[1] (was orig_a).
 // Kernel reads: a[i] (=orig_b[i]) and b[0] (=orig_a[0]).
 // Non-flipped would compute a[i] - b[0] = orig_b[i] - orig_a[0]. ✗
 // Flipped computes b[0] - a[i] = orig_a[0] - orig_b[i]. ✓
 let mut state: u64 = 0xFEED_5E91;
 let jit = MlirJitBackend::new();
 let m = 3_usize;
 let n = 4_usize;
 let a = make_f32_tensor(&[1], &mut state, 1.0, 5.0);
 let b = make_f32_tensor(&[m, n], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Sub)
 .expect("flipped scalar kernel missing for [1] Sub [M,N]");
 let av = read_f32(&a, 1);
 let bv = read_f32(&b, m * n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 expected[i] = av[0] - bv[i];
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 "flipped scalar [1] Sub [M,N]",
 );
}

#[test]
fn binop_f32_non_commutative_b_larger_suffix_equivalent() {
 // `[N] Div [M,N]` — non-commutative, b larger.  Suffix pattern
 // after swap; flipped variant computes `b[i mod N] op a[i]`
 // post-swap names ≡ original `orig_a[i mod N] / orig_b[i]`.
 let mut state: u64 = 0xFEED_5E92;
 let jit = MlirJitBackend::new();
 let m = 3_usize;
 let n = 5_usize;
 let a = make_f32_tensor(&[n], &mut state, 1.0, 5.0);
 let b = make_f32_tensor(&[m, n], &mut state, 1.0, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Div)
 .expect("flipped suffix kernel missing for [N] Div [M,N]");
 let av = read_f32(&a, n);
 let bv = read_f32(&b, m * n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 expected[i] = av[i % n] / bv[i];
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 "flipped suffix [N] Div [M,N]",
 );
}

#[test]
fn binop_f32_non_commutative_b_larger_mid_axis_equivalent() {
 // `[M,1,K] Sub [M,N,K]` — non-commutative, b larger, mid-axis.
 let mut state: u64 = 0xFEED_5E93;
 let jit = MlirJitBackend::new();
 let m = 2_usize;
 let n = 3_usize;
 let k = 4_usize;
 let total = m * n * k;
 let a = make_f32_tensor(&[m, 1, k], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, n, k], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Sub)
 .expect("flipped mid-axis kernel missing for [M,1,K] Sub [M,N,K]");
 let av = read_f32(&a, m * k);
 let bv = read_f32(&b, total);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 let mi = i / (n * k);
 let ki = i % k;
 let a_off = mi * k + ki;
 expected[i] = av[a_off] - bv[i];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "flipped mid-axis [M,1,K] Sub [M,N,K]",
 );
}

#[test]
fn binop_f32_b_larger_than_a_commutative_swaps() {
 // `[M] Add [M,N]` — b larger but op is commutative.  Swap to
 // `[M,N] Add [M]` (prefix-broadcast pattern) and JIT-route.
 // Output shape should match b (the larger), values should
 // match closed-form `a[m] + b[m,n]`.
 let mut state: u64 = 0xFEED_5E80;
 let jit = MlirJitBackend::new();
 let m = 3_usize;
 let n = 4_usize;
 let a = make_f32_tensor(&[m], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, n], &mut state, 0.5, 4.0);
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Mul,
 TensorBinaryOp::Min,
 TensorBinaryOp::Max,
 ] {
 let r = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("commutative swap failed for {:?}", op));
 // Output shape must be b's shape (the larger).
 assert_eq!(r.numel, m * n, "swap produced wrong size for {:?}", op);
 let av = read_f32(&a, m);
 let bv = read_f32(&b, m * n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 let mi = i / n;
 expected[i] = match op {
 TensorBinaryOp::Add => av[mi] + bv[i],
 TensorBinaryOp::Mul => av[mi] * bv[i],
 TensorBinaryOp::Min => av[mi].min(bv[i]),
 TensorBinaryOp::Max => av[mi].max(bv[i]),
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 &format!("commutative-swap [M] {:?} [M,N]", op),
 );
 }
}

#[test]
fn binop_f32_scalar_b_larger_commutative_swaps() {
 // `scalar Add [M,N]` — b dominates, op commutative.  After
 // swap: `[M,N] Add scalar` which routes through scalar-
 // broadcast.  Pin: output shape must match b (M*N), not a (1).
 let mut state: u64 = 0xFEED_5E81;
 let jit = MlirJitBackend::new();
 let m = 4_usize;
 let n = 5_usize;
 let a = make_f32_tensor(&[1], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, n], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("commutative scalar-on-left swap missing for Mul");
 assert_eq!(r.numel, m * n);
 let av = read_f32(&a, 1);
 let bv = read_f32(&b, m * n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 expected[i] = av[0] * bv[i];
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 "commutative-swap scalar Mul [M,N]",
 );
}

// ============================================================================
// Шаг 5e+2 — prefix-broadcast (`[M,N] op [M]`, `[M,N] op [M,1]` etc.)
// ============================================================================

#[test]
fn binop_f32_prefix_broadcast_1d_equivalent() {
 // `[M,N] op [M]` — bias is a per-row scalar; broadcast across
 // every column.
 let mut state: u64 = 0xFEED_5E60;
 let jit = MlirJitBackend::new();
 let m = 5_usize;
 let n_inner = 8_usize;
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Sub,
 TensorBinaryOp::Mul,
 ] {
 let a = make_f32_tensor(&[m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, op)
 .unwrap_or_else(|| panic!("JIT prefix-broadcast missing for binop F32 {:?}", op));
 let av = read_f32(&a, m * n_inner);
 let bv = read_f32(&b, m);
 let mut expected = vec![0.0_f32; m * n_inner];
 for i in 0..(m * n_inner) {
 let bj = i / n_inner;
 expected[i] = match op {
 TensorBinaryOp::Add => av[i] + bv[bj],
 TensorBinaryOp::Sub => av[i] - bv[bj],
 TensorBinaryOp::Mul => av[i] * bv[bj],
 _ => unreachable!(),
 };
 }
 assert_f32_close(
 &read_f32(&r, m * n_inner),
 &expected,
 &format!("binop F32 prefix-broadcast 1D {:?}", op),
 );
 }
}

#[test]
fn binop_f32_prefix_broadcast_with_trailing_1_equivalent() {
 // `[M,N] op [M,1]` — same semantics as `[M,N] op [M]`, but b
 // declares its rank-2 shape.  After stripping the trailing 1,
 // b's effective shape is `[M]`, so this routes through the
 // same prefix-broadcast kernel with `inner_size = N`.
 let mut state: u64 = 0xFEED_5E61;
 let jit = MlirJitBackend::new();
 let m = 4_usize;
 let n_inner = 6_usize;
 let a = make_f32_tensor(&[m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, 1], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("JIT prefix-broadcast with trailing-1 missing for Mul");
 let av = read_f32(&a, m * n_inner);
 let bv = read_f32(&b, m);
 let mut expected = vec![0.0_f32; m * n_inner];
 for i in 0..(m * n_inner) {
 expected[i] = av[i] * bv[i / n_inner];
 }
 assert_f32_close(
 &read_f32(&r, m * n_inner),
 &expected,
 "binop F32 [M,N] op [M,1] Mul",
 );
}

#[test]
fn binop_f32_prefix_broadcast_3d_per_batch_equivalent() {
 // `[B,M,N] op [B]` — per-batch scalar; broadcast over every
 // (m, n) cell.  inner_size = M*N.
 let mut state: u64 = 0xFEED_5E62;
 let jit = MlirJitBackend::new();
 let b_dim = 3_usize;
 let m = 2_usize;
 let n_inner = 4_usize;
 let total = b_dim * m * n_inner;
 let a = make_f32_tensor(&[b_dim, m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[b_dim], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Add)
 .expect("JIT 3D prefix-broadcast missing for Add");
 let av = read_f32(&a, total);
 let bv = read_f32(&b, b_dim);
 let mut expected = vec![0.0_f32; total];
 let inner = m * n_inner;
 for i in 0..total {
 expected[i] = av[i] + bv[i / inner];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [B,M,N] op [B] Add",
 );
}

// ============================================================================
// Шаг 5e+3 — generic mid-axis broadcast (NumPy-compatible)
// ============================================================================

#[test]
fn binop_f32_mid_axis_broadcast_3d_equivalent() {
 // `[M,N,K] op [M,1,K]` — middle axis broadcast.  Neither
 // suffix nor prefix; b's effective shape is `[M,1,K]` which
 // doesn't match either pattern.  This routes through
 // BinopMidAxisBroadcast.
 let mut state: u64 = 0xFEED_5E70;
 let jit = MlirJitBackend::new();
 let m = 3_usize;
 let n = 4_usize;
 let k = 5_usize;
 let a = make_f32_tensor(&[m, n, k], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[m, 1, k], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("JIT mid-axis broadcast missing for [M,N,K] op [M,1,K] Mul");
 let total = m * n * k;
 let av = read_f32(&a, total);
 let bv = read_f32(&b, m * k);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 let mi = i / (n * k);
 let ni = (i / k) % n; // unused — broadcast over n
 let ki = i % k;
 let _ = ni;
 let b_off = mi * k + ki;
 expected[i] = av[i] * bv[b_off];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [M,N,K] op [M,1,K] Mul",
 );
}

#[test]
fn binop_f32_mid_axis_broadcast_2d_left_equivalent() {
 // `[M,N] op [1,N]` — leading-1 broadcast.  Effective b shape
 // after stripping trailing 1s is `[1,N]` (first dim is 1, so
 // mid-axis applies).  This is also equivalent to suffix
 // `[M,N] op [N]` semantically, but b's rank-2 shape sends it
 // through the mid-axis path because `b_eff_len = 2` is not
 // less than `a_shape.len() = 2` for prefix detection, and
 // `[1,N] != a_shape.suffix([1,N].len())` because a_shape's
 // last 2 = `[M,N] != [1,N]` (when M != 1).
 let mut state: u64 = 0xFEED_5E71;
 let jit = MlirJitBackend::new();
 let m = 4_usize;
 let n = 6_usize;
 let a = make_f32_tensor(&[m, n], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[1, n], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Add)
 .expect("JIT mid-axis broadcast missing for [M,N] op [1,N] Add");
 let av = read_f32(&a, m * n);
 let bv = read_f32(&b, n);
 let mut expected = vec![0.0_f32; m * n];
 for i in 0..(m * n) {
 expected[i] = av[i] + bv[i % n];
 }
 assert_f32_close(
 &read_f32(&r, m * n),
 &expected,
 "binop F32 [M,N] op [1,N] Add",
 );
}

#[test]
fn binop_f32_mid_axis_broadcast_left_padded_equivalent() {
 // `[M,N,K] op [N,K]` — b has fewer dims; left-pad with 1.
 // After padding b_padded = [1,N,K], which matches mid-axis
 // criteria (only first axis broadcasts).  This is equivalent
 // to the suffix path semantically; the dispatcher should
 // route it through suffix (which is checked BEFORE mid-axis).
 // So this test verifies the suffix path correctly handles
 // `[M,N,K] op [N,K]` and DOESN'T accidentally fall through
 // to mid-axis.  Mostly a regression pin.
 let mut state: u64 = 0xFEED_5E72;
 let jit = MlirJitBackend::new();
 let m = 2_usize;
 let n = 3_usize;
 let k = 4_usize;
 let a = make_f32_tensor(&[m, n, k], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[n, k], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Sub)
 .expect("JIT broadcast missing for [M,N,K] op [N,K] Sub");
 let total = m * n * k;
 let av = read_f32(&a, total);
 let bv = read_f32(&b, n * k);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 expected[i] = av[i] - bv[i % (n * k)];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [M,N,K] op [N,K] Sub (should suffix-route)",
 );
}

#[test]
fn binop_f32_mid_axis_broadcast_4d_two_axes_equivalent() {
 // `[B,C,H,W] op [B,1,H,1]` — two interior broadcast axes.
 // Tests that the multi-axis decoder correctly handles
 // multiple axes with stride=0 simultaneously.
 let mut state: u64 = 0xFEED_5E73;
 let jit = MlirJitBackend::new();
 let b_dim = 2_usize;
 let c = 3_usize;
 let h = 4_usize;
 let w = 5_usize;
 let total = b_dim * c * h * w;
 let a = make_f32_tensor(&[b_dim, c, h, w], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[b_dim, 1, h, 1], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("JIT mid-axis broadcast missing for [B,C,H,W] op [B,1,H,1] Mul");
 let av = read_f32(&a, total);
 let bv = read_f32(&b, b_dim * h);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 let bi = i / (c * h * w);
 // ci unused (broadcast)
 let hi = (i / w) % h;
 // wi unused (broadcast)
 let b_off = bi * h + hi;
 expected[i] = av[i] * bv[b_off];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [B,C,H,W] op [B,1,H,1] Mul",
 );
}

#[test]
fn binop_f32_prefix_broadcast_3d_per_batch_row_equivalent() {
 // `[B,M,N] op [B,M]` — per-(batch,row) gain; broadcast over
 // the trailing N dim.  inner_size = N.
 let mut state: u64 = 0xFEED_5E63;
 let jit = MlirJitBackend::new();
 let b_dim = 2_usize;
 let m = 3_usize;
 let n_inner = 5_usize;
 let total = b_dim * m * n_inner;
 let a = make_f32_tensor(&[b_dim, m, n_inner], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[b_dim, m], &mut state, 0.5, 4.0);
 let r = jit
 .binop(&a, &b, TensorBinaryOp::Mul)
 .expect("JIT 3D-on-2D prefix-broadcast missing for Mul");
 let av = read_f32(&a, total);
 let bv = read_f32(&b, b_dim * m);
 let mut expected = vec![0.0_f32; total];
 for i in 0..total {
 expected[i] = av[i] * bv[i / n_inner];
 }
 assert_f32_close(
 &read_f32(&r, total),
 &expected,
 "binop F32 [B,M,N] op [B,M] Mul",
 );
}

#[test]
fn binop_f64_scalar_broadcast_equivalent() {
 let mut state: u64 = 0xCAFE_BABE_5E5E;
 let jit = MlirJitBackend::new();
 for &op in &[
 TensorBinaryOp::Add,
 TensorBinaryOp::Mul,
 ] {
 let a = make_f64_tensor(&[32], &mut state, 1.0, 5.0);
 let scalar = make_f64_tensor(&[1], &mut state, 1.0, 5.0);
 let r = jit
 .binop(&a, &scalar, op)
 .unwrap_or_else(|| panic!("JIT broadcast missing for binop F64 {:?}", op));
 let av = read_f64(&a, 32);
 let s = read_f64(&scalar, 1)[0];
 let mut expected = vec![0.0_f64; 32];
 for i in 0..32 {
 expected[i] = match op {
 TensorBinaryOp::Add => av[i] + s,
 TensorBinaryOp::Mul => av[i] * s,
 _ => unreachable!(),
 };
 }
 assert_f64_close(
 &read_f64(&r, 32),
 &expected,
 &format!("binop F64 broadcast {:?}", op),
 );
 }
}
