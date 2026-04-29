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
//! # Core Standard Library Validation Tests
//!
//! These tests ensure that ALL core/ .vr files parse and type-check correctly.
//! The stdlib is loaded during every compilation — these tests explicitly verify
//! that no parsing errors, type errors, or import resolution failures exist in
//! the standard library.
//!
//! Run with: `cargo test -p verum_compiler --test core_stdlib_validation_test`

use std::path::PathBuf;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// Verify that the stdlib loads without parse errors.
///
/// This test compiles a minimal program, which triggers stdlib loading.
/// Any parse failures in core/ .vr files will surface as compilation errors.
#[test]
fn test_core_stdlib_loads_without_errors() {
    let source = r#"
        fn main() -> Int {
            0
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("stdlib_check.vr"),
        output: PathBuf::from("stdlib_check"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    if let Err(e) = &result {
        // Display all diagnostics for debugging
        let _ = session.display_diagnostics();
        panic!(
            "Core stdlib loading failed: {}\n\
             This means one or more core/ .vr files have syntax or type errors.",
            e
        );
    }

    // Verify no stdlib errors were accumulated
    // Allow warnings but not errors
    let error_count = session.error_count();
    assert_eq!(
        error_count, 0,
        "Core stdlib produced {} errors during loading. \
         Run with RUST_LOG=debug for details.",
        error_count
    );
}

/// Count internal stdlib type registration errors.
///
/// These errors are suppressed during normal compilation but indicate
/// quality issues in the stdlib .vr files or type system coverage.
/// Requires a large stack due to deep type resolution of 280+ stdlib modules.
#[test]
fn test_count_stdlib_internal_type_errors() {
    // Spawn a thread with a large stack to avoid stack overflow
    let handle = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // 32 MB stack
        .spawn(|| {
            let options = CompilerOptions {
                input: PathBuf::from("stdlib_count.vr"),
                output: PathBuf::from("stdlib_count"),
                ..Default::default()
            };

            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            let (type_err, func_err, proto_err, impl_err, details) = pipeline.count_stdlib_type_errors();
            let total = type_err + func_err + proto_err + impl_err;

            eprintln!("=== Stdlib Internal Type Error Report ===");
            eprintln!("  Type resolution errors:     {}", type_err);
            eprintln!("  Function registration errors: {}", func_err);
            eprintln!("  Protocol registration errors: {}", proto_err);
            eprintln!("  Impl registration errors:     {}", impl_err);
            eprintln!("  TOTAL:                        {}", total);
            if !details.is_empty() {
                eprintln!("  --- Error Details ---");
                for detail in &details {
                    eprintln!("  {}", detail);
                }
            }
            eprintln!("=========================================");

            // Ratchet: assert total is at or below the documented
            // budget.  Any tightening (lowering the budget) is a
            // welcome PR; any regression (going above) gets
            // surfaced immediately so it can be diagnosed before it
            // accumulates.
            //
            // History:
            //   2026-03-26  budget: 10  (count: 6)   — Args/Output
            //                                          context-order issues
            //   2026-04-29  budget: 75  (count: 75)  — accumulated drift,
            //                                          mostly TypeNotFound
            //                                          for path-prefixes
            //                                          `base.X` / `common.X`
            //                                          that should resolve
            //                                          to core.{base,common}
            //                                          (registration order
            //                                          / module-path
            //                                          resolution issue in
            //                                          fn/impl registration
            //                                          phase, NOT in
            //                                          user code)
            //   2026-04-29  budget: 5   (count: 5)   — `base.X` / `common.X`
            //                                          path-prefix call sites
            //                                          rewritten to bare names
            //                                          via brace-mount
            //                                          (52 .vr files touched);
            //                                          `Event` mismatch in
            //                                          core/term renamed to
            //                                          match its consumers;
            //                                          `AlterValidationError`
            //                                          import name corrected;
            //                                          remaining 5 are the
            //                                          `Output` registration-
            //                                          order bug in
            //                                          core/io/process.vr
            //                                          (tracked separately —
            //                                          requires compiler
            //                                          fix, not stdlib fix)
            //
            // Reduce as the Output registration-order bug lands.
            const BUDGET: usize = 5;
            assert!(
                total <= BUDGET,
                "Stdlib internal type errors ({total}) exceed budget ({BUDGET}). \
                 New regression — investigate the most recent .vr / type-system \
                 changes.  See test_count_stdlib_internal_type_errors history \
                 in this file.",
            );
        })
        .expect("Failed to spawn thread");

    handle.join().expect("Thread panicked");
}

/// Count all stdlib body type-checking errors.
///
/// This runs the FULL type checker (including function bodies) on all core/ modules
/// to identify all type errors. This is the comprehensive test that exposes the
/// ~950 errors that were previously hidden by skipping stdlib body checking.
#[test]
#[ignore = "takes ~220s; run with: RUST_MIN_STACK=134217728 cargo test -p verum_compiler --test core_stdlib_validation_test test_count_stdlib_body_errors -- --ignored --nocapture"]
fn test_count_stdlib_body_errors() {
    let handle = std::thread::Builder::new()
        .stack_size(128 * 1024 * 1024) // 128 MB stack
        .spawn(|| {
            let options = CompilerOptions {
                input: PathBuf::from("stdlib_body_check.vr"),
                output: PathBuf::from("stdlib_body_check"),
                ..Default::default()
            };

            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            let (total, categories, details) = pipeline.count_stdlib_body_errors();

            eprintln!("=== Stdlib Body Type Error Report ===");
            eprintln!("  TOTAL errors: {}", total);
            eprintln!("  --- By Category ---");
            let mut sorted_cats: Vec<_> = categories.iter().collect();
            sorted_cats.sort_by(|a, b| b.1.cmp(a.1));
            for (cat, count) in &sorted_cats {
                eprintln!("    {}: {}", cat, count);
            }
            if !details.is_empty() {
                eprintln!("  --- Error Details (all) ---");
                for detail in details.iter() {
                    eprintln!("  {}", detail);
                }
                // Print summary line (always last)
                if let Some(summary) = details.last()
                    && summary.starts_with("[SUMMARY]") {
                        eprintln!("  {}", summary);
                    }
            }
            eprintln!("=====================================");
        })
        .expect("Failed to spawn thread");

    handle.join().expect("Thread panicked");
}

/// Verify that basic programs with core types compile through VBC.
///
/// Uses simple types (Int, Text, Bool) that are always available
/// without generic resolution. Generic collections (List, Map, Set)
/// require the full AOT pipeline to resolve.
#[test]
fn test_core_basic_types_available() {
    let source = r#"
        fn add(a: Int, b: Int) -> Int {
            a + b
        }

        fn main() -> Int {
            let x: Int = 42;
            let y: Bool = true;
            let z: Float = 3.14;
            add(x, 1)
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("types_check.vr"),
        output: PathBuf::from("types_check"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    if let Err(e) = &result {
        let _ = session.display_diagnostics();
        panic!(
            "Core basic types not available: {}\n\
             Ensure core/base/ modules load correctly.",
            e
        );
    }
}

/// Verify that CBGR reference types parse and compile.
///
/// Tests that the three-tier reference system (&T, &checked T, &unsafe T)
/// is properly wired through the compilation pipeline.
#[test]
fn test_cbgr_reference_types_compile() {
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let source = r#"
                fn takes_ref(x: &Int) -> Int {
                    *x
                }

                fn takes_checked_ref(x: &checked Int) -> Int {
                    *x
                }

                fn main() -> Int {
                    let x = 42;
                    let r = &x;
                    takes_ref(r)
                }
            "#;

            let options = CompilerOptions {
                input: PathBuf::from("cbgr_check.vr"),
                output: PathBuf::from("cbgr_check"),
                ..Default::default()
            };

            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            let result = pipeline.compile_string(source);

            if let Err(e) = &result {
                let _ = session.display_diagnostics();
                panic!(
                    "CBGR reference types failed to compile: {}\n\
                     Check parser support for &T, &checked T, &unsafe T.",
                    e
                );
            }
        })
        .expect("Failed to spawn thread")
        .join();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// Verify that functions with &mut self compile through VBC.
///
/// This exercises the CBGR RefMut instruction path in the VBC codegen.
#[test]
fn test_mut_ref_compiles() {
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let source = r#"
                fn increment(x: &mut Int) {
                    *x = *x + 1;
                }

                fn main() -> Int {
                    let mut val = 41;
                    increment(&mut val);
                    val
                }
            "#;

            let options = CompilerOptions {
                input: PathBuf::from("mut_ref_check.vr"),
                output: PathBuf::from("mut_ref_check"),
                ..Default::default()
            };

            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            let result = pipeline.compile_string(source);

            if let Err(e) = &result {
                let _ = session.display_diagnostics();
                panic!(
                    "Mutable reference compilation failed: {}\n\
                     Check CBGR RefMut instruction emission.",
                    e
                );
            }
        })
        .expect("Failed to spawn thread")
        .join();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
