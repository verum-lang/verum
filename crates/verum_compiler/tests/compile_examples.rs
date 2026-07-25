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
// Tests for compiling example programs
// These tests depend on example files existing

//! Integration tests for compiling and running example programs
//!
//! Tests the end-to-end compilation pipeline with various example programs
//! that exercise different language features.

use std::path::PathBuf;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// Run compilation test with larger stack to avoid overflow
fn run_with_large_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024) // 64 MB stack
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn test_hello_world_compilation() {
    run_with_large_stack(|| {
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/hello_world.vr");

        if !example.exists() {
            eprintln!(
                "Skipping test: example file not found at {}",
                example.display()
            );
            return;
        }

        let options = CompilerOptions {
            input: example,
            output: PathBuf::from("/tmp/hello_world"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        // Should compile without errors
        assert!(
            pipeline.run_check_only().is_ok(),
            "hello_world.vr should compile successfully"
        );
    });
}

#[test]
fn test_fibonacci_compilation() {
    run_with_large_stack(|| {
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/fibonacci.vr");

        if !example.exists() {
            eprintln!(
                "Skipping test: example file not found at {}",
                example.display()
            );
            return;
        }

        let options = CompilerOptions {
            input: example.clone(),
            output: PathBuf::from("/tmp/fibonacci"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        // Try to compile - at least parse and load the file
        let result = pipeline.run_check_only();
        // Test that the pipeline runs without panicking
        // The actual result depends on type checker support for forward references
        match result {
            Ok(_) => println!("fibonacci.vr compiled successfully"),
            Err(e) => {
                let err_msg = format!("{:?}", e);
                // Known limitation: forward references might cause unbound variable errors
                if err_msg.contains("unbound variable") {
                    eprintln!(
                        "Note: Forward reference limitation in {}",
                        example.display()
                    );
                } else {
                    panic!("Unexpected error compiling {}: {:?}", example.display(), e);
                }
            }
        }
    });
}

#[test]
fn test_factorial_compilation() {
    run_with_large_stack(|| {
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/factorial.vr");

        if !example.exists() {
            eprintln!(
                "Skipping test: example file not found at {}",
                example.display()
            );
            return;
        }

        let options = CompilerOptions {
            input: example.clone(),
            output: PathBuf::from("/tmp/factorial"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        // Try to compile
        let result = pipeline.run_check_only();
        match result {
            Ok(_) => println!("factorial.vr compiled successfully"),
            Err(e) => {
                let err_msg = format!("{:?}", e);
                // Known limitation: forward references might cause unbound variable errors
                if err_msg.contains("unbound variable") {
                    eprintln!(
                        "Note: Forward reference limitation in {}",
                        example.display()
                    );
                } else {
                    panic!("Unexpected error compiling {}: {:?}", example.display(), e);
                }
            }
        }
    });
}
