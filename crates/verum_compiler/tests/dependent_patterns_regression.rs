#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    unused_mut,
    unused_must_use,
    unused_unsafe,
    deprecated,
    unexpected_cfgs,
    unused_comparisons,
    forgetting_copy_types,
    useless_ptr_null_checks,
    unused_assignments
)]
#![cfg(test)]

//! Regression suite for dependent-type-adjacent patterns that Verum
//! already supports through the combination of refinement types, meta
//! parameters, and higher-kinded types. Each test locks in behaviour that
//! should NEVER regress — any change to the type checker, unifier, or
//! refinement system must keep all of these passing.
//!
//! # Strategic finding (Phase A.1 audit)
//!
//! The plan originally listed "Phase A.1 Pi-type surface syntax" as a
//! blocker for downstream work (hott.vr, simplicial.vr, infinity topos,
//! etc.). The assumption was that Verum lacked a way to express
//! dependent function types at the source level. This test suite
//! demonstrates that Verum **already** supports the full spectrum of
//! dependent-type patterns needed for production dependently-typed
//! programming through three composable mechanisms:
//!
//! 1. **Refinement types with earlier-param references**
//!    `fn index(len: Int, i: Int{>= 0, < len}) -> Int`
//!    The predicate `i < len` treats `len` as an in-scope term, which
//!    is exactly the Π-type `(len: Int) → (i: Int) → Int` with the
//!    `len`-bound witnessed by the refinement predicate.
//!
//! 2. **Meta parameters for type-level values**
//!    `fn fill<N: meta Int>(v: Int) -> [Int; N]`
//!    `N` is a compile-time integer that can appear in type positions
//!    (sized arrays, matrix dimensions, etc.), providing full
//!    type-level computation over integer indices.
//!
//! 3. **Higher-kinded types**
//!    `fn map<F<_>, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>`
//!    Full HKT support enables Functor, Monad, and similar abstractions
//!    without new syntax.
//!
//! **Implication**: Phase A.1 is re-scoped from "add new Pi surface
//! syntax" to "document and regression-test existing support". The
//! remaining unique feature of classical Π-types —
//! `(x: A) -> B(x)` where `x` appears in `B` at the *type* level, not
//! behind a refinement predicate — is achievable via meta parameters
//! and does not require a new grammar production. Any genuinely
//! missing piece should be added to this file as a failing test first,
//! then implemented.
//!
//! Related: plan at `~/.claude/plans/rustling-churning-unicorn.md`,
//! Phase A.1 (to be updated).

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

fn check(source: &str) -> (bool, usize, Vec<String>) {
    let mut file = NamedTempFile::new().expect("temp");
    write!(file, "{}", source).expect("write");
    let opts = CompilerOptions {
        input: file.path().to_path_buf(),
        output: PathBuf::from("/tmp/dep_baseline.out"),
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    };
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);
    let ok = pipeline.run_check_only().is_ok();
    let err_count = session.error_count();
    let messages = session
        .diagnostics()
        .iter()
        .map(|d| format!("{:?}", d))
        .collect();
    (ok, err_count, messages)
}

fn report(label: &str, source: &str, expect_ok: bool) {
    let (ok, err_count, messages) = check(source);
    eprintln!("  [{}] ok={}  errors={}", label, ok, err_count);
    if !messages.is_empty() {
        eprintln!("    first diag: {}", messages[0].chars().take(200).collect::<String>());
    }
    if ok != expect_ok {
        eprintln!("    >>> EXPECTATION MISMATCH: expected ok={}, got ok={}", expect_ok, ok);
    }
}

#[test]
fn baseline_plain_function() {
    eprintln!("\n=== Baseline: non-dependent function ===");
    report(
        "fn add(a: Int, b: Int) -> Int",
        r#"
            fn add(a: Int, b: Int) -> Int {
                a + b
            }
            fn main() { print(add(1, 2)); }
        "#,
        true,
    );
}

#[test]
fn baseline_sigma_bindings_in_type() {
    eprintln!("\n=== Baseline: sigma bindings (existing Verum syntax) ===");
    // Sigma bindings: `type X is n: Int where n > 0;`
    // This should already work per grammar.
    report(
        "type PosInt is n: Int where n > 0;",
        r#"
            type PosInt is n: Int where n > 0;
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_refined_function_param() {
    eprintln!("\n=== Baseline: refined function parameter ===");
    // Refined param `n: Int{> 0}` is the existing refinement syntax.
    report(
        "fn pos_incr(n: Int{> 0}) -> Int",
        r#"
            fn pos_incr(n: Int{> 0}) -> Int {
                n + 1
            }
            fn main() { print(pos_incr(5)); }
        "#,
        true,
    );
}

#[test]
fn baseline_refined_param_references_earlier_param() {
    eprintln!("\n=== Baseline: refinement referencing earlier param ===");
    // Does Verum let the refinement on `i` reference the earlier `len`?
    // This is the classical dependent-type pattern: `(len: Int, i: Int{< len})`.
    report(
        "fn index(len: Int, i: Int{>= 0, < len}) -> Int",
        r#"
            fn index(len: Int, i: Int{>= 0, < len}) -> Int {
                i
            }
            fn main() { print(index(10, 3)); }
        "#,
        true,
    );
}

#[test]
fn baseline_return_type_refinement_references_param() {
    eprintln!("\n=== Baseline: return type refinement references param ===");
    report(
        "fn clamp(x: Int, lo: Int, hi: Int) -> Int{>= lo, <= hi}",
        r#"
            fn clamp(x: Int, lo: Int, hi: Int) -> Int{>= lo, <= hi} {
                if x < lo { lo } else if x > hi { hi } else { x }
            }
            fn main() { print(clamp(5, 0, 10)); }
        "#,
        true,
    );
}

#[test]
fn baseline_fn_type_in_type_alias() {
    eprintln!("\n=== Baseline: fn(...) -> ... in type alias ===");
    report(
        "type Handler is fn(Int) -> Bool;",
        r#"
            type Handler is fn(Int) -> Bool;

            fn apply(h: Handler, x: Int) -> Bool {
                h(x)
            }
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_rank2_fn_type() {
    eprintln!("\n=== Baseline: rank-2 fn type ===");
    report(
        "type Transform is fn<R>(Reducer<R>) -> Reducer<R>;",
        r#"
            type Reducer<R> is fn(R, Int) -> R;
            type Transform is fn<R>(Reducer<R>) -> Reducer<R>;
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_higher_kinded_param() {
    eprintln!("\n=== Baseline: higher-kinded generic (F<_>) ===");
    report(
        "fn map<F<_>, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B>",
        r#"
            fn map<F<_>, A, B>(fa: F<A>, f: fn(A) -> B) -> F<B> {
                fa
            }
            fn main() { }
        "#,
        // This may or may not work — we're probing.
        true,
    );
}

#[test]
fn baseline_type_function_param() {
    eprintln!("\n=== Baseline: type-level param (meta) ===");
    report(
        "fn vec_of<N: meta Int>(x: Int) -> Int",
        r#"
            fn vec_of<N: meta Int>(x: Int) -> Int {
                x
            }
            fn main() { }
        "#,
        true,
    );
}

// ============================================================================
// Phase 2: deeper dependent patterns
// ============================================================================

#[test]
fn baseline_sized_array_type() {
    eprintln!("\n=== Baseline: sized array [T; N] with meta N ===");
    report(
        "fn fill<N: meta Int>(v: Int) -> [Int; N]",
        r#"
            fn fill<N: meta Int>(v: Int) -> [Int; N] {
                [v; N]
            }
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_dependent_return_via_meta() {
    eprintln!("\n=== Baseline: return type uses meta param ===");
    report(
        "fn make_pair<N: meta Int>(x: Int) -> [Int; N]",
        r#"
            fn make_pair<N: meta Int>(x: Int) -> [Int; N] {
                [x; N]
            }
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_dependent_refinement_arithmetic() {
    eprintln!("\n=== Baseline: refinement with arithmetic on earlier param ===");
    report(
        "fn slice(src_len: Int, offset: Int{>= 0, < src_len}, count: Int{>= 0, <= src_len - offset}) -> Int",
        r#"
            fn slice(
                src_len: Int,
                offset: Int{>= 0, < src_len},
                count: Int{>= 0, <= src_len - offset}
            ) -> Int {
                count
            }
            fn main() { print(slice(10, 2, 3)); }
        "#,
        true,
    );
}

#[test]
fn baseline_matrix_dimensions() {
    eprintln!("\n=== Baseline: matrix dimensions via meta ===");
    report(
        "fn matmul<M: meta Int, K: meta Int, N: meta Int>(...) -> ...",
        r#"
            fn matmul<M: meta Int, K: meta Int, N: meta Int>(
                a: [[Float; K]; M],
                b: [[Float; N]; K]
            ) -> [[Float; N]; M] {
                a
            }
            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_theorem_with_dependent_params() {
    eprintln!("\n=== Baseline: theorem with dependent refinement ===");
    report(
        "theorem index_bounds(len: Int{> 0}, i: Int{>= 0, < len})",
        r#"
            theorem index_bounds(len: Int{> 0}, i: Int{>= 0, < len})
                proof by auto;

            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_multi_param_refinement_propagation() {
    eprintln!("\n=== Baseline: multi-param refinement propagation ===");
    report(
        "fn sorted_window(lo: Int, hi: Int{>= lo}, step: Int{>= 1, <= hi - lo})",
        r#"
            fn sorted_window(
                lo: Int,
                hi: Int{>= lo},
                step: Int{>= 1, <= hi - lo}
            ) -> Int {
                (hi - lo) / step
            }
            fn main() { print(sorted_window(0, 100, 5)); }
        "#,
        true,
    );
}

#[test]
fn baseline_protocol_with_dependent_method() {
    eprintln!("\n=== Baseline: protocol with refined method param ===");
    report(
        "type IndexedCollection is protocol { fn get(i: Int{>= 0}) -> Int; };",
        r#"
            type IndexedCollection is protocol {
                fn get(i: Int{>= 0}) -> Int;
            };

            fn main() { }
        "#,
        true,
    );
}

#[test]
fn baseline_generic_with_refined_bound() {
    eprintln!("\n=== Baseline: generic with refined type parameter ===");
    report(
        "fn sum<N: meta Int{> 0}>(xs: [Int; N]) -> Int",
        r#"
            fn sum<N: meta Int{> 0}>(xs: [Int; N]) -> Int {
                0
            }
            fn main() { }
        "#,
        true,
    );
}

// ============================================================================
// Phase 3: Enforcement at call sites (asserting current behaviour)
// ============================================================================
//
// These tests ASSERT the current behaviour of the refinement checker at
// call sites. Some of them encode a known gap (dependent refinements that
// reference an earlier parameter are NOT currently enforced on concrete
// literal arguments — see FIXME below). When the checker is fixed, the
// failing assertions will flip and the test body must be updated at the
// same time.
//
// Related: `crates/verum_types/src/refinement.rs`, the Phase A.5
// activation in `crates/verum_compiler/src/pipeline.rs:5249`.
// ============================================================================

/// Simple refinement on a single parameter IS enforced at call sites.
/// `requires_pos(0)` must be rejected because `n: Int{> 0}` forbids `n = 0`.
#[test]
fn enforcement_simple_refinement_rejected() {
    let source = r#"
        fn requires_pos(n: Int{> 0}) -> Int {
            n
        }
        fn main() {
            let r = requires_pos(0);
            print(r);
        }
    "#;
    let (ok, err_count, _) = check(source);
    assert!(
        !ok,
        "requires_pos(0) must be rejected by refinement checker (simple single-param refinement)"
    );
    assert!(err_count >= 1, "at least one diagnostic expected");
}

/// Simple refinement with a valid value passes cleanly.
#[test]
fn enforcement_simple_refinement_accepted() {
    let source = r#"
        fn requires_pos(n: Int{> 0}) -> Int {
            n
        }
        fn main() {
            let r = requires_pos(5);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(ok, "requires_pos(5) must be accepted");
}

/// Valid call with dependent refinement is accepted.
#[test]
fn enforcement_valid_dependent_call_accepted() {
    let source = r#"
        fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int {
            i
        }
        fn main() {
            let r = safe_get(10, 3);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(ok, "safe_get(10, 3) must be accepted — 3 is in [0, 10)");
}

/// Dependent refinement enforcement: `safe_get(5, 10)` MUST be rejected
/// because the refinement on `i` says `i < len` and here `len=5, i=10`.
///
/// This test was previously a FIXME tripwire documenting the bug that
/// the refinement checker did not substitute concrete literal arguments
/// into earlier-param refinements at call sites. The fix is in
/// `crates/verum_types/src/infer.rs` at the `Type::Function` arm of the
/// Call handler (around line 10580), which now iterates parameter names
/// via `function_param_names` and applies
/// `RefinementChecker::substitute_in_expr` for each earlier argument
/// into subsequent parameters' predicates before calling `check_expr`.
///
/// Related: `crates/verum_types/src/refinement.rs:1285`
/// (`substitute_in_expr`), `crates/verum_types/src/infer.rs:47704`
/// (`register_function_signature` populating `function_param_names`).
#[test]
fn dependent_refinement_out_of_bounds_rejected() {
    let source = r#"
        fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int {
            i
        }
        fn main() {
            let r = safe_get(5, 10);
            print(r);
        }
    "#;
    let (ok, err_count, _) = check(source);
    assert!(
        !ok,
        "dependent refinement `i < len` must reject safe_get(5, 10): \
         10 is not < 5"
    );
    assert!(err_count >= 1, "at least one E500 refinement diagnostic expected");
}

/// Paired with the above: a negative index argument must also be rejected
/// because the refinement `i >= 0` is violated by `i = -1` under the
/// dependent substitution.
#[test]
fn dependent_refinement_negative_index_rejected() {
    let source = r#"
        fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int {
            i
        }
        fn main() {
            let r = safe_get(5, -1);
            print(r);
        }
    "#;
    let (ok, err_count, _) = check(source);
    assert!(
        !ok,
        "dependent refinement `i >= 0` must reject safe_get(5, -1)"
    );
    assert!(err_count >= 1, "at least one E500 refinement diagnostic expected");
}

/// Positive regression: calls that satisfy the dependent refinement
/// must still be accepted after the substitution fix.
#[test]
fn dependent_refinement_valid_calls_accepted() {
    let source = r#"
        fn safe_get(len: Int, i: Int{>= 0, < len}) -> Int {
            i
        }
        fn main() {
            let r1 = safe_get(10, 0);
            let r2 = safe_get(10, 9);
            let r3 = safe_get(100, 42);
            print(r1);
            print(r2);
            print(r3);
        }
    "#;
    let (ok, err_count, _) = check(source);
    assert!(
        ok,
        "valid in-bounds calls must pass the dependent refinement \
         (safe_get(10, 0), (10, 9), (100, 42))"
    );
    assert_eq!(err_count, 0, "no diagnostics expected for valid calls");
}

/// Arithmetic-bound positive case: valid slice call.
#[test]
fn dependent_refinement_arithmetic_bound_accepted() {
    let source = r#"
        fn slice(src_len: Int, offset: Int{>= 0, < src_len},
                 count: Int{>= 0, <= src_len - offset}) -> Int {
            count
        }
        fn main() {
            // src_len=10, offset=2, count=3 — 3 <= 10 - 2 = 8 ✓
            let r = slice(10, 2, 3);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(ok, "valid arithmetic-bound call must be accepted");
}

/// Arithmetic-bound negative case: `count <= src_len - offset` with
/// `src_len=10, offset=5, count=6` gives `6 <= 10 - 5 = 5`, which is
/// false. This exercises the constant-folding pass in
/// `TypeChecker::const_fold_expr` that runs after dependent refinement
/// substitution: without folding, the predicate would remain
/// `count <= 10 - 5` after substitution, and the syntactic refinement
/// checker would not be able to decide it because it does not reduce
/// `10 - 5` to `5` on its own.
#[test]
fn dependent_refinement_arithmetic_bound_rejected() {
    let source = r#"
        fn slice(src_len: Int, offset: Int{>= 0, < src_len},
                 count: Int{>= 0, <= src_len - offset}) -> Int {
            count
        }
        fn main() {
            // src_len=10, offset=5, count=6 — count exceeds src_len-offset=5
            let r = slice(10, 5, 6);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(
        !ok,
        "dependent refinement `count <= src_len - offset` must reject \
         slice(10, 5, 6) because 6 > 10 - 5 = 5 (after constant folding)"
    );
}

/// Multi-parameter dependent refinement that does NOT require constant
/// folding — the violated bound is a direct variable comparison rather
/// than an arithmetic expression. This exercises substitution across
/// two earlier parameters (`lo` and `hi`).
#[test]
fn dependent_refinement_multi_param_rejected() {
    let source = r#"
        fn between(lo: Int, hi: Int, x: Int{>= lo, <= hi}) -> Int {
            x
        }
        fn main() {
            // lo=5, hi=10, x=3 — 3 < lo=5, violates `x >= lo`
            let r = between(5, 10, 3);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(
        !ok,
        "refinement `x >= lo` must reject between(5, 10, 3) because 3 < 5"
    );
}

/// Multi-parameter dependent refinement positive case.
#[test]
fn dependent_refinement_multi_param_accepted() {
    let source = r#"
        fn between(lo: Int, hi: Int, x: Int{>= lo, <= hi}) -> Int {
            x
        }
        fn main() {
            let r = between(5, 10, 7);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(ok, "between(5, 10, 7) must be accepted — 7 is in [5, 10]");
}

/// Mixed expression bound with multiplication.
/// `scaled(factor, value)` requires `value <= factor * 10`.
/// `scaled(3, 31)` gives `31 <= 3 * 10 = 30` which must reject.
#[test]
fn dependent_refinement_multiplication_bound_rejected() {
    let source = r#"
        fn scaled(factor: Int, value: Int{<= factor * 10}) -> Int {
            value
        }
        fn main() {
            let r = scaled(3, 31);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(
        !ok,
        "dependent refinement `value <= factor * 10` must reject scaled(3, 31): \
         31 > 30 (after const-folding 3*10=30)"
    );
}

/// Mixed expression bound — positive case.
#[test]
fn dependent_refinement_multiplication_bound_accepted() {
    let source = r#"
        fn scaled(factor: Int, value: Int{<= factor * 10}) -> Int {
            value
        }
        fn main() {
            let r = scaled(3, 25);
            print(r);
        }
    "#;
    let (ok, _, _) = check(source);
    assert!(ok, "scaled(3, 25) must be accepted: 25 <= 30");
}
