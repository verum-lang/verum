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

use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session, VerifyMode};

/// Helper to create a temp file with Verum source code
fn create_temp_source(source: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("Failed to create temp file");
    write!(file, "{}", source).expect("Failed to write temp file");
    file
}

#[test]
fn test_compile_simple_function() {
    let source = r#"
        fn add(a: Int, b: Int) -> Int {
            a + b
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        verify_mode: VerifyMode::Runtime,
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Should compile successfully
    let result = pipeline.run_check_only();
    assert!(
        result.is_ok(),
        "Failed to type check simple function: {:?}",
        result
    );

    // Should have no errors
    assert!(!session.has_errors(), "Session has errors");
    assert_eq!(session.error_count(), 0);
}

#[test]
fn test_compile_fibonacci() {
    let source = r#"
        fn fibonacci(n: Int) -> Int {
            match n {
                0 => 0,
                1 => 1,
                n => fibonacci(n - 1) + fibonacci(n - 2)
            }
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(result.is_ok(), "Failed to compile fibonacci: {:?}", result);
}

#[test]
fn test_compile_factorial_with_refinement() {
    let source = r#"
        fn factorial(n: Int) -> Int {
            match n {
                0 => 1,
                n => n * factorial(n - 1)
            }
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        verify_mode: VerifyMode::Auto,
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(result.is_ok(), "Failed to compile factorial: {:?}", result);
}

#[test]
fn test_type_error_detection() {
    let source = r#"
        fn bad_add(a: Int, b: Bool) -> Int {
            a + b
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Should fail due to type error
    let result = pipeline.run_check_only();
    assert!(result.is_err(), "Should have detected type error");

    // Should have at least one error
    assert!(session.has_errors(), "Should have errors");
    assert!(session.error_count() > 0, "Should have error count > 0");
}

#[test]
fn test_parse_error_recovery() {
    let source = r#"
        fn incomplete(a: Int {
            // Missing closing brace and body
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    // Should fail to parse
    let result = pipeline.run_check_only();
    assert!(result.is_err(), "Should fail on parse error");
}

#[test]
fn test_multiple_functions() {
    let source = r#"
        fn double(x: Int) -> Int {
            x * 2
        }

        fn triple(x: Int) -> Int {
            x * 3
        }

        fn sum_transformed(a: Int, b: Int) -> Int {
            double(a) + triple(b)
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(
        result.is_ok(),
        "Failed to compile multiple functions: {:?}",
        result
    );
}

#[test]
fn test_tuple_types() {
    let source = r#"
        fn swap(pair: (Int, Bool)) -> (Bool, Int) {
            let (a, b) = pair;
            (b, a)
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(
        result.is_ok(),
        "Failed to compile tuple function: {:?}",
        result
    );
}

#[test]
fn test_let_bindings() {
    let source = r#"
        fn compute(x: Int) -> Int {
            let y = x * 2;
            let z = y + 10;
            z * 3
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(
        result.is_ok(),
        "Failed to compile let bindings: {:?}",
        result
    );
}

#[test]
fn test_if_expression() {
    let source = r#"
        fn abs(x: Int) -> Int {
            if x < 0 {
                -x
            } else {
                x
            }
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(
        result.is_ok(),
        "Failed to compile if expression: {:?}",
        result
    );
}

#[test]
fn test_empty_module() {
    let source = r#""#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let result = pipeline.run_check_only();
    assert!(result.is_ok(), "Empty module should compile");
}

#[test]
fn test_session_diagnostics() {
    let source = r#"
        fn test() -> Int {
            let x: Bool = 42;  // Type error
            x
        }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let mut session = Session::new(opts);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let _ = pipeline.run_check_only();

    // Check that diagnostics are being collected
    assert!(session.has_errors(), "Should have type error");
    let diagnostics = session.diagnostics();
    assert!(!diagnostics.is_empty(), "Should have diagnostics");
}

#[test]
fn test_compiler_options_builder() {
    let opts = CompilerOptions::new("test.vr".into(), "test.out".into())
        .with_verify_mode(VerifyMode::Proof)
        .with_optimization(3)
        .with_verification_costs(true);

    assert_eq!(opts.verify_mode, VerifyMode::Proof);
    assert_eq!(opts.optimization_level, 3);
    assert!(opts.show_verification_costs);
    assert!(opts.is_release());
    assert!(!opts.is_debug());
}

#[test]
fn test_session_caching() {
    let source = r#"
        fn test() -> Int { 42 }
    "#;

    let temp_file = create_temp_source(source);
    let opts = CompilerOptions {
        input: temp_file.path().to_path_buf(),
        output: PathBuf::from("/tmp/test.out"),
        ..Default::default()
    };

    let session = Session::new(opts);

    // First load
    let file_id = session.load_file(temp_file.path()).unwrap();
    let source1 = session.get_source(file_id);
    assert!(source1.is_some());

    // Second load should use cache
    let file_id2 = session.load_file(temp_file.path()).unwrap();
    assert_eq!(file_id, file_id2, "Should return same file ID");

    let stats = session.stats();
    assert_eq!(stats.num_files, 1, "Should have only one file loaded");
}
