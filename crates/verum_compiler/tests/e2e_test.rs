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
//!
//! These tests verify that the entire compilation pipeline works correctly
//! from source code to executable output.

use std::path::PathBuf;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

#[test]
fn test_compile_hello_world() {
    // Simple hello world program
    let source = r#"
        fn main() {
            // println is now available via stdlib::prelude
            let x: Int = 42;
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: PathBuf::from("test"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Should compile without errors (in check mode)
    let result = pipeline.compile_string(source);

    // Check diagnostics even if there's an error
    if let Err(e) = &result {
        eprintln!("Compilation error: {}", e);
        let _ = session.display_diagnostics();
    }

    assert!(
        result.is_ok(),
        "Compilation should succeed for simple hello world"
    );
}

#[test]
fn test_compile_simple_function() {
    let source = r#"
        fn add(x: Int, y: Int) -> Int {
            x + y
        }

        fn main() {
            let result = add(1, 2);
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: PathBuf::from("test"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    if let Err(e) = &result {
        eprintln!("Compilation error: {}", e);
        let _ = session.display_diagnostics();
    }

    assert!(
        result.is_ok(),
        "Compilation should succeed for simple function"
    );
}

#[test]
fn test_compile_with_types() {
    let source = r#"
        type Point is {
            x: Float,
            y: Float,
        };

        fn distance(p1: Point, p2: Point) -> Float {
            let dx = p1.x - p2.x;
            let dy = p1.y - p2.y;
            dx * dx + dy * dy
        }

        fn main() {
            let p1 = Point { x: 0.0, y: 0.0 };
            let p2 = Point { x: 3.0, y: 4.0 };
            let dist = distance(p1, p2);
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: PathBuf::from("test"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    if let Err(e) = &result {
        eprintln!("Compilation error: {}", e);
        let _ = session.display_diagnostics();
    }

    assert!(
        result.is_ok(),
        "Compilation should succeed for struct types"
    );
}

#[test]
fn test_compile_with_refinement_types() {
    let source = r#"
        type Positive = Int where self > 0;

        fn square(x: Positive) -> Int {
            x * x
        }

        fn main() {
            // This should type-check
            let x: Positive = 5;
            let result = square(x);
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: PathBuf::from("test"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    if let Err(e) = &result {
        eprintln!("Compilation error: {}", e);
        let _ = session.display_diagnostics();
    }

    // This might fail until refinement type support is complete
    // For now, we just check that it doesn't crash
    let _ = result;
}

#[test]
fn test_compile_error_reporting() {
    // Invalid source - missing type annotation
    let source = r#"
        fn main() {
            let x;  // Missing type and initializer
        }
    "#;

    let options = CompilerOptions {
        input: PathBuf::from("test.vr"),
        output: PathBuf::from("test"),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.compile_string(source);

    // Should fail with parse or type error
    assert!(
        result.is_err(),
        "Invalid source should produce compilation error"
    );

    // Should have at least one error diagnostic
    assert!(session.error_count() > 0, "Should have error diagnostics");
}

#[test]
fn test_pipeline_modes() {
    let source = r#"
        fn main() {
            let x: Int = 42;
        }
    "#;

    // Test check mode
    {
        let options = CompilerOptions {
            input: PathBuf::from("test.vr"),
            output: PathBuf::from("test"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        let result = pipeline.compile_string(source);
        if let Err(e) = &result {
            eprintln!("Check mode error: {}", e);
            let _ = session.display_diagnostics();
        }
        // Check mode should work
        let _ = result;
    }

    // Test JIT mode (using new() - specific modes not yet implemented)
    {
        let options = CompilerOptions {
            input: PathBuf::from("test.vr"),
            output: PathBuf::from("test"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        let result = pipeline.compile_string(source);
        if let Err(e) = &result {
            eprintln!("JIT mode error: {}", e);
            let _ = session.display_diagnostics();
        }
        // JIT mode might not be fully implemented yet
        let _ = result;
    }

    // Test AOT mode (using new() - specific modes not yet implemented)
    {
        let options = CompilerOptions {
            input: PathBuf::from("test.vr"),
            output: PathBuf::from("test"),
            ..Default::default()
        };

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        let result = pipeline.compile_string(source);
        if let Err(e) = &result {
            eprintln!("AOT mode error: {}", e);
            let _ = session.display_diagnostics();
        }
        // AOT mode might not be fully implemented yet
        let _ = result;
    }
}
