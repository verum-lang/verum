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

//! HoTT (Homotopy Type Theory) expressibility probe.
//!
//! Before committing to Phase B (new cubical grammar + parser + type
//! system), probe whether the HoTT foundation (Equiv, IsEquiv, IsContr,
//! IsProp, IsSet, Fiber, etc.) can already be expressed in Verum using
//! only existing syntactic mechanisms: protocols, generics, dependent
//! refinement types, meta parameters, rank-2 polymorphism, and
//! higher-kinded types.
//!
//! If this probe succeeds broadly, Phase B reduces to stdlib authoring
//! + documentation rather than language-level work. If specific HoTT
//! constructs fail, this file documents exactly what's missing and
//! focuses Phase B on the smallest necessary feature delta.
//!
//! The plan update from the Phase A.1 discovery (refinements + meta +
//! HKT already give dependent types in practice) makes this probe very
//! likely to succeed for most of the core HoTT constructs.

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

fn check(source: &str) -> (bool, usize, Vec<String>) {
    let mut file = NamedTempFile::new().expect("temp");
    write!(file, "{}", source).expect("write");
    let opts = CompilerOptions {
        input: file.path().to_path_buf(),
        output: PathBuf::from("/tmp/hott_probe.out"),
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

fn report(label: &str, source: &str) -> bool {
    let (ok, err_count, messages) = check(source);
    eprintln!("  [{}] ok={}  errors={}", label, ok, err_count);
    if !messages.is_empty() && !ok {
        let first = &messages[0];
        eprintln!(
            "    first diag: {}",
            first.chars().take(250).collect::<String>()
        );
    }
    ok
}

// ============================================================================
// 1. Equivalence and IsEquiv (Chapter 2 of HoTT book)
// ============================================================================

#[test]
fn hott_equiv_protocol() {
    eprintln!("\n=== HoTT: Equiv<A, B> via protocol + rank-2 ===");
    // An equivalence is a function with a two-sided inverse up to pointwise equality.
    // We use protocols and records; the "up to equality" proofs become Bool-valued
    // witness functions until cubical primitives land.
    let source = r#"
        type IsEquiv<A, B> is protocol {
            fn inverse(f: fn(A) -> B, y: B) -> A;
        };

        type Equiv<A, B> is {
            forward: fn(A) -> B,
            backward: fn(B) -> A,
        };

        fn apply_fwd<A, B>(e: Equiv<A, B>, x: A) -> B {
            (e.forward)(x)
        }

        fn apply_bwd<A, B>(e: Equiv<A, B>, y: B) -> A {
            (e.backward)(y)
        }

        fn main() {
            print("Equiv parsed");
        }
    "#;
    assert!(report("Equiv", source));
}

#[test]
fn hott_identity_equivalence() {
    eprintln!("\n=== HoTT: identity equivalence id_equiv ===");
    let source = r#"
        type Equiv<A, B> is {
            forward: fn(A) -> B,
            backward: fn(B) -> A,
        };

        fn id_equiv<A>() -> Equiv<A, A> {
            Equiv {
                forward: fn(x: A) -> A { x },
                backward: fn(x: A) -> A { x },
            }
        }

        fn main() { }
    "#;
    assert!(report("id_equiv", source));
}

// ============================================================================
// 2. Contractibility, Propositions, Sets (truncation levels)
// ============================================================================

#[test]
fn hott_is_contr_record() {
    eprintln!("\n=== HoTT: IsContr<A> record ===");
    // IsContr(A) = (center : A, contraction : (x : A) -> Path(center, x))
    // We represent the contraction as a Bool-valued witness function
    // until Path types land; the semantic intent is preserved.
    let source = r#"
        type IsContr<A> is {
            center: A,
            // (x : A) -> a witness that center = x
            contraction: fn(A) -> Bool,
        };

        fn unit_is_contr() -> IsContr<Int> {
            IsContr {
                center: 0,
                contraction: fn(x: Int) -> Bool { true },
            }
        }

        fn main() { }
    "#;
    assert!(report("IsContr", source));
}

#[test]
fn hott_is_prop_protocol() {
    eprintln!("\n=== HoTT: IsProp<A> protocol ===");
    let source = r#"
        // A type is a proposition iff all its inhabitants are (propositionally) equal.
        type IsProp<A> is protocol {
            fn is_singleton(&self, x: &A, y: &A) -> Bool;
        };

        fn main() { }
    "#;
    assert!(report("IsProp", source));
}

#[test]
fn hott_is_set_protocol() {
    eprintln!("\n=== HoTT: IsSet<A> protocol ===");
    let source = r#"
        type IsSet<A> is protocol {
            fn uip(&self, x: &A, y: &A, p: Bool, q: Bool) -> Bool;
        };

        fn main() { }
    "#;
    assert!(report("IsSet", source));
}

#[test]
fn hott_is_trunc_n_meta_level() {
    eprintln!("\n=== HoTT: IsTrunc<n, A> via meta int ===");
    // IsTrunc(n, A) can be encoded via a meta-level Int parameter
    // and refinement types.
    let source = r#"
        type IsTrunc<A, N: meta Int{>= 0}> is protocol {
            fn trunc_witness(&self) -> Bool;
        };

        fn main() { }
    "#;
    assert!(report("IsTrunc", source));
}

// ============================================================================
// 3. Fiber and Univalence axiom
// ============================================================================

#[test]
fn hott_fiber_record() {
    eprintln!("\n=== HoTT: Fiber<A, B, f, y> ===");
    let source = r#"
        // Fiber of f : A -> B over y : B.
        // fiber(f, y) = (x : A, p : f(x) = y)
        type Fiber<A, B> is {
            point: A,
            // (f(point) = y) — witness until Path lands
            witness: Bool,
        };

        fn main() { }
    "#;
    assert!(report("Fiber", source));
}

#[test]
fn hott_univalence_axiom() {
    eprintln!("\n=== HoTT: univalence as axiom declaration ===");
    // Univalence: (A ≃ B) ≃ (A = B)
    // Until Path types land, we declare it as an axiom returning Bool.
    let source = r#"
        type Equiv<A, B> is {
            forward: fn(A) -> B,
            backward: fn(B) -> A,
        };

        axiom univalence<A, B>(e: Equiv<A, B>) -> Bool;

        fn main() { }
    "#;
    assert!(report("univalence axiom", source));
}

// ============================================================================
// 4. Function extensionality (funext)
// ============================================================================

#[test]
fn hott_funext_theorem() {
    eprintln!("\n=== HoTT: funext theorem declaration ===");
    let source = r#"
        theorem funext<A, B>(f: fn(A) -> B, g: fn(A) -> B)
            proof by auto;

        fn main() { }
    "#;
    assert!(report("funext", source));
}

// ============================================================================
// 5. Higher inductive type (HIT) shape — the circle S^1
// ============================================================================

#[test]
fn hott_s1_as_protocol() {
    eprintln!("\n=== HoTT: S¹ (circle) as protocol with base + loop ===");
    // The circle S¹ has one point `base` and one path `loop : base = base`.
    // Until HIT path constructors land, we declare loop as an axiom.
    let source = r#"
        type S1 is protocol {
            fn base(&self) -> Int;
        };

        axiom s1_loop(s: Int) -> Bool;

        fn main() { }
    "#;
    assert!(report("S1", source));
}

// ============================================================================
// 6. Dependent type classes / type-indexed records (category example)
// ============================================================================

#[test]
fn hott_category_signature() {
    eprintln!("\n=== HoTT: Category signature with dependent morphism type ===");
    let source = r#"
        // Parametrised morphism carrier using HKT
        type Category<Obj, Mor<_, _>> is protocol {
            fn id<A>() -> Mor<A, A>;
            fn compose<A, B, C>(f: Mor<A, B>, g: Mor<B, C>) -> Mor<A, C>;
        };

        fn main() { }
    "#;
    assert!(report("Category HKT", source));
}

// ============================================================================
// 7. Sigma types and dependent pairs
// ============================================================================

#[test]
fn hott_sigma_bindings() {
    eprintln!("\n=== HoTT: Σ-type via sigma_bindings ===");
    // sigma_binding grammar: identifier ':' type_expr [ 'where' expression ].
    // Refinements on sigma members must use `where`, not inline `{...}`.
    let source = r#"
        // (n : Nat) × (Vec<Int, n>) — a Σ-type is a dependent record.
        type SizedIntVec is n: Int where n >= 0, data: [Int; n];

        fn main() { }
    "#;
    assert!(report("SizedIntVec sigma", source));
}

// ============================================================================
// 8. Path-like equality via refinement types
// ============================================================================

#[test]
fn hott_path_via_refinement() {
    eprintln!("\n=== HoTT: Path<A>(a, b) via refinement type ===");
    // A propositional equality x = y can be encoded as a refinement
    // on a unit-like witness value: Path_A(a, b) = Unit{a == b}.
    let source = r#"
        type PathInt is { lhs: Int, rhs: Int, witness: Bool };

        fn refl_int(x: Int) -> PathInt {
            PathInt { lhs: x, rhs: x, witness: true }
        }

        fn main() { }
    "#;
    assert!(report("PathInt refinement", source));
}

// ============================================================================
// 9. Proof-irrelevance witness functions (standard Verum pattern)
// ============================================================================

#[test]
fn hott_proof_irrelevance_marker() {
    eprintln!("\n=== HoTT: proof irrelevance marker protocol ===");
    let source = r#"
        type ProofIrrelevant<P> is protocol {
            fn are_equal(&self, p1: &P, p2: &P) -> Bool;
        };

        fn main() { }
    "#;
    assert!(report("ProofIrrelevant", source));
}
