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
// Integration tests for the compilation pipeline
// Tests the full compilation process from source to execution
//
// These tests must run serially because they change the working directory
// which is global state shared across all threads.
//
// NOTE: Updated to use verum_compiler (the unified compiler) instead of
// the old verum_cli::compiler module.

use serial_test::serial;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use verum_cli::config::Manifest;
use verum_compiler::options::CompilerOptions;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// Helper to create a test project
fn create_test_project() -> TempDir {
    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir(&src_dir).unwrap();

    // Create a manifest file (use Verum.toml for proper detection)
    let manifest_content = r#"
[cog]
name = "test_project"
version = "0.1.0"
edition = "2024"

[build]
target = "native"

[language]
profile = "application"
"#;
    fs::write(temp.path().join("Verum.toml"), manifest_content).unwrap();

    temp
}

/// Helper to run code in a temp directory context.
///
/// Runs the closure on a thread with a 64 MB stack to accommodate deep
/// recursion during stdlib loading and type inference.
fn with_temp_project<F>(f: F)
where
    F: FnOnce(PathBuf) + Send + 'static,
{
    let temp = create_test_project();
    let temp_path = temp.path().to_path_buf();

    // Get current directory, falling back to home if current dir doesn't exist
    let old_dir = std::env::current_dir()
        .or_else(|_| {
            dirs::home_dir()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home"))
        })
        .expect("Failed to get current or home directory");

    let handle = std::thread::Builder::new()
        .name("compilation-test".to_string())
        .stack_size(64 * 1024 * 1024) // 64 MB stack for stdlib processing
        .spawn(move || {
            // Change to temp directory so compiler can find the manifest
            std::env::set_current_dir(&temp_path).expect("Failed to change to temp directory");

            // Use a guard to ensure we always restore the directory
            struct DirGuard(PathBuf);
            impl Drop for DirGuard {
                fn drop(&mut self) {
                    let _ = std::env::set_current_dir(&self.0);
                }
            }
            let _guard = DirGuard(old_dir);

            // Run the test closure
            f(temp_path);
        })
        .expect("Failed to spawn test thread");

    handle.join().expect("Test thread panicked");
    // temp is dropped here, cleaning up the directory after the thread finishes
}

#[test]
#[serial]
fn test_check_compilation() {
    with_temp_project(|project_path| {
        // Write a simple Verum source file
        let source = r#"fn main() {
    let x = 42;
    let y = x + 1;
    print(f"y = {y}");
}
"#;
        fs::write(project_path.join("src").join("main.vr"), source).unwrap();

        // Create compiler session with check mode
        let mut options = CompilerOptions::default();
        options.input = project_path.join("src");
        options.check_only = true;

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new_check(&mut session);

        // Run type checking
        let result = pipeline.check_project();
        assert!(
            result.is_ok(),
            "Check compilation failed: {:?}",
            result.err()
        );

        let check_result = result.unwrap();
        assert!(check_result.files_checked > 0, "No files were checked");
        assert_eq!(check_result.errors, 0, "There should be no errors");
    });
}

#[test]
#[serial]
#[ignore = "stdlib type mismatch: Ok(Unit)|Err(OSError) vs Ok(Unit)|Overflow(Unit)"]
fn test_compile_project() {
    with_temp_project(|project_path| {
        // Write a Verum source file with functions
        let source = r#"fn factorial(n: Int) -> Int {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

fn main() -> Int {
    factorial(5)
}
"#;
        fs::write(project_path.join("src").join("main.vr"), source).unwrap();

        // Create compiler session
        let mut options = CompilerOptions::default();
        options.input = project_path.join("src");

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        // Compile project
        let result = pipeline.compile_project();
        assert!(result.is_ok(), "Compilation failed: {:?}", result.err());
    });
}

#[test]
#[serial]
fn test_native_compilation() {
    with_temp_project(|project_path| {
        // Write a simple Verum source file (avoid for loops due to codegen limitations)
        let source = r#"fn add(a: Int, b: Int) -> Int {
    a + b
}

fn main() -> Int {
    add(10, 20)
}
"#;
        fs::write(project_path.join("src").join("main.vr"), source).unwrap();

        // Create compiler session with AOT mode
        let mut options = CompilerOptions::default();
        options.input = project_path.join("src").join("main.vr");
        options.optimization_level = 2;

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        // Native compilation
        let result = pipeline.run_native_compilation();
        assert!(
            result.is_ok(),
            "Native compilation failed: {:?}",
            result.err()
        );

        // Check that executable was created
        let output_path = result.unwrap();
        assert!(
            output_path.exists(),
            "Executable was not created at {:?}",
            output_path
        );
    });
}

#[test]
#[serial]
#[ignore = "stdlib type mismatch: Ok(Unit)|Err(OSError) vs Ok(Unit)|Overflow(Unit)"]
fn test_multi_file_compilation() {
    with_temp_project(|project_path| {
        // Write multiple source files
        let lib_source = r#"pub fn add(a: Int, b: Int) -> Int {
    a + b
}

pub fn multiply(a: Int, b: Int) -> Int {
    a * b
}
"#;
        fs::write(project_path.join("src").join("lib.vr"), lib_source).unwrap();

        let main_source = r#"fn main() -> Int {
    let result = 10 + 20;
    result
}
"#;
        fs::write(project_path.join("src").join("main.vr"), main_source).unwrap();

        // Compile project
        let mut options = CompilerOptions::default();
        options.input = project_path.join("src");

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new(&mut session);

        let result = pipeline.compile_project();
        assert!(
            result.is_ok(),
            "Multi-file compilation failed: {:?}",
            result.err()
        );
    });
}

#[test]
#[serial]
fn test_compilation_with_errors() {
    with_temp_project(|project_path| {
        // Write source with syntax error
        let source = r#"fn main( {
    let x = 42;
}
"#;
        fs::write(project_path.join("src").join("main.vr"), source).unwrap();

        // Create compiler session
        let mut options = CompilerOptions::default();
        options.input = project_path.join("src");
        options.continue_on_error = true;

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new_check(&mut session);

        // This should fail due to syntax error
        let result = pipeline.check_project();

        // We expect either an error or a check result with errors
        match result {
            Ok(check_result) => {
                assert!(check_result.errors > 0, "Expected compilation errors");
            }
            Err(_) => {
                // Also acceptable - the pipeline returned an error
            }
        }
    });
}

#[test]
#[serial]
fn test_type_checking() {
    with_temp_project(|project_path| {
        // Write source that should type check successfully
        let source = r#"fn double(x: Int) -> Int {
    x * 2
}

fn main() {
    let value = 5;
    let result = double(value);
    // print expects Text, so use string interpolation
    print(f"Result: {result}");
}
"#;
        fs::write(project_path.join("src").join("main.vr"), source).unwrap();

        let mut options = CompilerOptions::default();
        options.input = project_path.join("src");

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new_check(&mut session);

        let result = pipeline.check_project();
        assert!(result.is_ok(), "Type checking failed: {:?}", result.err());

        let check_result = result.unwrap();
        if check_result.errors > 0 {
            // Display diagnostics to see what the errors are
            let _ = session.display_diagnostics();
        }
        assert_eq!(check_result.errors, 0, "Expected no type errors");
    });
}
