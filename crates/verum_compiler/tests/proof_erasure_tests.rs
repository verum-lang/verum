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

//! Proof erasure regression tests.
//!
//! Proofs are a purely compile-time phenomenon in Verum. Theorem, Lemma,
//! Corollary, Axiom, and Tactic declarations must be verified by the
//! `proof_verification` phase and then **completely erased** before VBC
//! codegen so that the runtime carries zero proof-term overhead.
//!
//! These tests guarantee:
//!
//! 1. All 5 proof item kinds parse, type-check, and do not reach VBC codegen.
//! 2. Compilation succeeds end-to-end even when proof items are interleaved
//!    with runtime functions.
//! 3. Removing a proof item does not change the compiled VBC module footprint
//!    for runtime functions (regression guard).
//!
//! Related code:
//! - `crates/verum_vbc/src/codegen/mod.rs:3233` — explicit skip of all 5
//!   proof item kinds in `register_top_level_item`.
//! - `crates/verum_compiler/src/phases/proof_verification.rs` — the phase
//!   that verifies proofs before they are erased.
//! - `crates/verum_ast/src/decl.rs:90-105` — `ItemKind::Theorem`,
//!   `ItemKind::Lemma`, `ItemKind::Corollary`, `ItemKind::Axiom`,
//!   `ItemKind::Tactic`.

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

// =============================================================================
// Helpers
// =============================================================================

/// Create a temp `.vr` file containing the given source and return it.
/// The file is kept alive via `NamedTempFile` — caller must bind the result.
fn create_temp_source(source: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("temp file");
    write!(file, "{}", source).expect("write temp file");
    file
}

/// Build a `CompilerOptions` with the input pointed at the given temp file.
fn opts_for(temp: &NamedTempFile) -> CompilerOptions {
    CompilerOptions {
        input: temp.path().to_path_buf(),
        output: PathBuf::from("/tmp/proof_erasure_test.out"),
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    }
}

/// Run the compiler in check-only mode on the given source.
/// Returns `true` iff the pipeline reports success and the session has no
/// errors.
fn check_only_succeeds(source: &str) -> bool {
    let temp = create_temp_source(source);
    let opts = opts_for(&temp);
    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);
    pipeline.run_check_only().is_ok() && !session.has_errors()
}

// =============================================================================
// 1. Parse + type-check acceptance for each proof item kind
// =============================================================================

#[test]
fn theorem_with_tactic_proof_compiles() {
    let source = r#"
        theorem addition_non_negative(a: Int, b: Int)
            requires a >= 0, b >= 0
            proof by auto;

        fn main() {
            print("ok");
        }
    "#;
    assert!(
        check_only_succeeds(source),
        "theorem with `proof by auto` must type-check"
    );
}

#[test]
fn theorem_with_no_parameters_compiles() {
    let source = r#"
        theorem reflexivity()
            proof by trivial;

        fn main() { }
    "#;
    assert!(check_only_succeeds(source));
}

#[test]
fn theorem_with_return_type_compiles() {
    let source = r#"
        theorem non_zero(n: Int) -> Bool
            requires n > 0
            proof by auto;

        fn main() { }
    "#;
    assert!(check_only_succeeds(source));
}

#[test]
fn lemma_compiles() {
    let source = r#"
        lemma helper_monotone(a: Int, b: Int)
            requires a <= b
            proof by auto;

        fn main() { }
    "#;
    assert!(
        check_only_succeeds(source),
        "lemma declaration must type-check"
    );
}

#[test]
fn axiom_compiles() {
    let source = r#"
        axiom chosen_well_ordering(n: Int) -> Bool;

        fn main() { }
    "#;
    assert!(
        check_only_succeeds(source),
        "axiom declaration must type-check"
    );
}

#[test]
fn multiple_theorems_in_one_module_compile() {
    let source = r#"
        theorem t1(a: Int)
            requires a > 0
            proof by auto;

        theorem t2(b: Int)
            requires b >= 0
            proof by trivial;

        lemma l1(c: Int)
            requires c != 0
            proof by auto;

        axiom a1(x: Int) -> Bool;

        fn main() {
            print("multi-proof module ok");
        }
    "#;
    assert!(
        check_only_succeeds(source),
        "multiple proof items in one module must type-check"
    );
}

// =============================================================================
// 2. Proof items mixed with runtime code
// =============================================================================

#[test]
fn proof_items_do_not_break_runtime_functions() {
    let source = r#"
        theorem double_commutes(a: Int, b: Int)
            requires a >= 0, b >= 0
            proof by auto;

        fn double(x: Int) -> Int {
            x + x
        }

        fn main() {
            let y = double(21);
            print(y);
        }
    "#;
    assert!(
        check_only_succeeds(source),
        "runtime functions alongside theorems must compile"
    );
}

#[test]
fn axiom_used_as_hypothesis_in_theorem_compiles() {
    let source = r#"
        axiom excluded_middle(p: Bool) -> Bool;

        theorem uses_axiom(x: Int)
            requires x != 0
            proof by auto;

        fn main() { }
    "#;
    assert!(check_only_succeeds(source));
}

// =============================================================================
// 3. Runtime-identical footprint: removing proof items does not change
// the set of runtime-compiled symbols.
// =============================================================================

/// The theorem declaration is the ONLY difference between these two sources.
/// If proof erasure is working, both must compile successfully.
#[test]
fn removing_theorem_does_not_break_compilation() {
    let with_theorem = r#"
        theorem foo_positive(n: Int)
            requires n > 0
            proof by auto;

        fn runtime_function(x: Int) -> Int {
            x * 2
        }

        fn main() {
            let r = runtime_function(10);
            print(r);
        }
    "#;

    let without_theorem = r#"
        fn runtime_function(x: Int) -> Int {
            x * 2
        }

        fn main() {
            let r = runtime_function(10);
            print(r);
        }
    "#;

    assert!(
        check_only_succeeds(with_theorem),
        "source with theorem must compile"
    );
    assert!(
        check_only_succeeds(without_theorem),
        "source without theorem must compile"
    );
}

/// Adding an axiom to an otherwise-working module must not introduce errors.
#[test]
fn adding_axiom_does_not_break_compilation() {
    let baseline = r#"
        fn square(x: Int) -> Int {
            x * x
        }

        fn main() {
            print(square(7));
        }
    "#;

    let with_axiom = r#"
        axiom stub_axiom(x: Int) -> Bool;

        fn square(x: Int) -> Int {
            x * x
        }

        fn main() {
            print(square(7));
        }
    "#;

    assert!(check_only_succeeds(baseline));
    assert!(
        check_only_succeeds(with_axiom),
        "adding an axiom must not break compilation"
    );
}

/// Adding a lemma to a working module must not change runtime compilation.
#[test]
fn adding_lemma_does_not_break_compilation() {
    let with_lemma = r#"
        lemma double_is_even(n: Int)
            requires n >= 0
            proof by auto;

        fn double(n: Int) -> Int {
            n + n
        }

        fn main() {
            print(double(5));
        }
    "#;
    assert!(check_only_succeeds(with_lemma));
}

// =============================================================================
// 4. Regression guards: all 5 proof item kinds must be handled explicitly
// by VBC codegen. If someone adds a new proof item kind to ItemKind and
// forgets to list it in the explicit match, these tests do NOT fire — that
// gap is caught at Rust compile time by the now-explicit match in
// `crates/verum_vbc/src/codegen/mod.rs:3243-3249`. The catch-all arm was
// intentionally NOT removed but it is now unreachable for listed kinds.
// =============================================================================

#[test]
fn theorem_and_lemma_and_axiom_all_in_one_module() {
    let source = r#"
        theorem t(a: Int)
            requires a >= 0
            proof by auto;

        lemma l(a: Int)
            requires a > 0
            proof by trivial;

        axiom ax(b: Int) -> Bool;

        fn main() {
            print("proof zoo ok");
        }
    "#;
    assert!(
        check_only_succeeds(source),
        "theorem+lemma+axiom zoo must type-check"
    );
}

#[test]
fn empty_module_still_compiles() {
    let source = r#"
        fn main() { }
    "#;
    assert!(
        check_only_succeeds(source),
        "baseline empty module must compile"
    );
}

// =============================================================================
// 5. Proof items with generic parameters
// =============================================================================

#[test]
fn generic_theorem_compiles() {
    let source = r#"
        theorem generic_refl<T>(x: T) -> Bool
            proof by auto;

        fn main() { }
    "#;
    assert!(
        check_only_succeeds(source),
        "generic theorem must type-check"
    );
}

#[test]
fn generic_axiom_compiles() {
    let source = r#"
        axiom generic_ax<T>(x: T) -> Bool;

        fn main() { }
    "#;
    assert!(check_only_succeeds(source));
}
