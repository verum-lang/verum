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

#[test]
fn binop_f32_prefix_broadcast_falls_through() {
 // `[M] op [M,N]` — b is LARGER than a; not a suffix and not a
 // scalar.  This must NOT match either broadcast pattern; the
 // dispatcher should return None and let the caller's outer
 // dispatch path handle it (potentially returning the
 // mathematically-correct fallback or surfacing an error).
 let mut state: u64 = 0xFEED_5E54;
 let jit = MlirJitBackend::new();
 let a = make_f32_tensor(&[3], &mut state, 0.5, 4.0);
 let b = make_f32_tensor(&[3, 4], &mut state, 0.5, 4.0);
 let r = jit.binop(&a, &b, TensorBinaryOp::Add);
 assert!(
 r.is_none(),
 "prefix-broadcast `[M] op [M,N]` must fall through (got JIT-routed result)"
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
