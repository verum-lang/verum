//! Tests for the check_project() method in the compilation pipeline.
//!
//! This tests multi-file type checking without code generation.

use anyhow::Result;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// Test check_project() with a simple single-file project
#[test]
fn test_check_project_single_file() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let main_file = temp_dir.path().join("main.vr");

    // Write a simple Verum file
    std::fs::write(
        &main_file,
        r#"
fn add(a: Int, b: Int) -> Int {
    a + b
}

fn main() -> Int {
    add(10, 20)
}
"#,
    )?;

    // Create session
    let options = CompilerOptions {
        input: main_file.clone(),
        output: PathBuf::new(),
        verbose: 0,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    // Run check_project
    let result = pipeline.check_project()?;

    // Verify results
    assert_eq!(result.files_checked, 1, "Should check 1 file");
    assert!(result.types_inferred > 0, "Should infer some types");
    assert_eq!(result.errors, 0, "Should have no errors");
    assert!(result.is_ok(), "Check should succeed");

    println!("Check result: {}", result.summary());

    Ok(())
}

/// Test check_project() with multiple files
#[test]
fn test_check_project_multi_file() -> Result<()> {
    let temp_dir = TempDir::new()?;

    // Create main.vr
    let main_file = temp_dir.path().join("main.vr");
    std::fs::write(
        &main_file,
        r#"
fn add(a: Int, b: Int) -> Int {
    a + b
}

fn main() -> Int {
    add(5, 10)
}
"#,
    )?;

    // Create utils.vr (separate file, no imports for simplicity)
    let utils_file = temp_dir.path().join("utils.vr");
    std::fs::write(
        &utils_file,
        r#"
pub fn multiply(a: Int, b: Int) -> Int {
    a * b
}

pub fn square(x: Int) -> Int {
    multiply(x, x)
}
"#,
    )?;

    // Create session
    let options = CompilerOptions {
        input: main_file.clone(),
        output: PathBuf::new(),
        verbose: 0,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    // Run check_project
    let result = pipeline.check_project()?;

    // Verify results
    assert_eq!(result.files_checked, 2, "Should check 2 files");
    assert!(result.types_inferred > 0, "Should infer some types");

    println!("Multi-file check result: {}", result.summary());

    Ok(())
}

/// Test check_project() with empty directory
#[test]
fn test_check_project_empty() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let dummy_file = temp_dir.path().join("dummy.txt");
    std::fs::write(&dummy_file, "not a verum file")?;

    // Create session
    let options = CompilerOptions {
        input: dummy_file.clone(),
        output: PathBuf::new(),
        verbose: 0,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    // Run check_project
    let result = pipeline.check_project()?;

    // Verify results
    assert_eq!(result.files_checked, 0, "Should check 0 files");
    assert_eq!(result.types_inferred, 0, "Should infer 0 types");
    assert!(result.is_ok(), "Check should succeed (vacuously)");

    Ok(())
}

/// Test check_project() with type errors
#[test]
fn test_check_project_with_errors() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let main_file = temp_dir.path().join("main.vr");

    // Write a Verum file with type errors
    std::fs::write(
        &main_file,
        r#"
fn add(a: Int, b: Int) -> Int {
    a + b
}

fn main() -> Int {
    // Type error: passing Text to function expecting Int
    add("hello", "world")
}
"#,
    )?;

    // Create session
    let options = CompilerOptions {
        input: main_file.clone(),
        output: PathBuf::new(),
        verbose: 0,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    // Run check_project - should not panic, but may have errors
    let result = pipeline.check_project();

    // The check itself should succeed (not crash), but may report errors
    match result {
        Ok(check_result) => {
            println!("Check completed with result: {}", check_result.summary());
            assert_eq!(check_result.files_checked, 1, "Should check 1 file");
        }
        Err(e) => {
            // It's also acceptable to fail during parsing/type checking
            println!("Check failed (expected): {}", e);
        }
    }

    Ok(())
}
