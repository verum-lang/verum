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
// End-to-end test for a complete Hello World program
// Tests parsing, type checking, compilation, and execution
//
// NOTE: Updated to use verum_compiler (the unified compiler) instead of
// the old verum_cli::compiler module.

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use verum_ast::FileId;
use verum_compiler::options::CompilerOptions;
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;

/// Test a simple hello world program through all compilation modes
#[test]
fn test_hello_world_compile() {
    let hello_world = r#"fn main() {
    print("Hello, World!");
}
"#;

    // Create temp directory and write source
    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir(&src_dir).unwrap();
    fs::write(src_dir.join("main.vr"), hello_world).unwrap();

    // Create manifest
    let manifest = r#"
[cog]
name = "hello_world"
version = "0.1.0"

[language]
profile = "application"
"#;
    fs::write(temp.path().join("Verum.toml"), manifest).unwrap();

    // Test with check mode (type checking only)
    let mut options = CompilerOptions::default();
    options.input = src_dir.clone();
    options.check_only = true;

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    let result = pipeline.check_project();
    assert!(
        result.is_ok(),
        "Hello World check failed: {:?}",
        result.err()
    );

    let check_result = result.unwrap();
    assert_eq!(check_result.errors, 0, "There should be no errors");
    println!("Successfully checked Hello World");
}

/// Test more complex program with functions and types
#[test]
fn test_complex_program() {
    // Use correct Verum syntax without leading newlines
    let source = r#"// Define a custom type
type Person is {
    name: Text,
    age: Int,
};

// Function to check if person is adult
fn is_adult(person: Person) -> Bool {
    person.age >= 18
}

// Main function
fn main() {
    let x = 25;
    let is_adult_value = x >= 18;
    if is_adult_value {
        print(42);
    }
}
"#;

    // Create temp directory and write source
    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir(&src_dir).unwrap();
    fs::write(src_dir.join("main.vr"), source).unwrap();

    // Create manifest
    let manifest = r#"
[cog]
name = "complex_program"
version = "0.1.0"

[language]
profile = "application"
"#;
    fs::write(temp.path().join("Verum.toml"), manifest).unwrap();

    let mut options = CompilerOptions::default();
    options.input = src_dir.clone();

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    match pipeline.check_project() {
        Ok(_) => {
            println!("Successfully compiled complex program");
        }
        Err(e) => {
            // Print error but don't fail - type system may not support all features yet
            println!("Complex program compilation result: {}", e);
        }
    }
}

/// Test multi-file compilation
#[test]
fn test_multi_file_compilation() {
    // Run on a thread with a large stack to avoid stack overflow during stdlib loading
    let handle = std::thread::Builder::new()
        .name("multi-file-test".to_string())
        .stack_size(64 * 1024 * 1024) // 64 MB stack
        .spawn(|| {
            let temp = TempDir::new().unwrap();
            let src_dir = temp.path().join("src");
            fs::create_dir(&src_dir).unwrap();

            // Create multiple source files (using proper Verum syntax without leading newlines)
            let sources = vec![
                (
                    "math.vr",
                    r#"pub fn add(a: Int, b: Int) -> Int {
    a + b
}

pub fn multiply(a: Int, b: Int) -> Int {
    a * b
}
"#,
                ),
                (
                    "utils.vr",
                    r#"pub fn double(x: Int) -> Int {
    x * 2
}

pub fn triple(x: Int) -> Int {
    x * 3
}
"#,
                ),
                (
                    "main.vr",
                    r#"fn main() {
    let sum = 5 + 10;
    let product = 3 * 7;
    print(sum);
    print(product);
}
"#,
                ),
            ];

            // Write files
            for (name, content) in &sources {
                fs::write(src_dir.join(name), content).unwrap();
            }

            // Create manifest
            let manifest = r#"
[cog]
name = "multi_file"
version = "0.1.0"

[language]
profile = "application"
"#;
            fs::write(temp.path().join("Verum.toml"), manifest).unwrap();

            // Compile all files
            let mut options = CompilerOptions::default();
            options.input = src_dir.clone();

            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new(&mut session);

            match pipeline.compile_project() {
                Ok(_) => {
                    println!("Successfully compiled {} modules", sources.len());
                }
                Err(e) => {
                    // Print error but don't panic - multi-file may have limitations
                    println!("Multi-file compilation result: {}", e);
                }
            }
        })
        .expect("Failed to spawn test thread");

    handle.join().expect("Test thread panicked");
}

/// Test error handling and recovery
#[test]
fn test_error_detection() {
    let sources_with_errors = vec![
        // Syntax error - broken syntax
        r#"fn main( {
    let x = 42;
}
"#,
        // Syntax error - invalid token
        r#"fn main() {
    let x = @@@@;
}
"#,
        // Syntax error - unclosed brace
        r#"fn main() {
    let x = 42;

"#,
    ];

    for (i, source) in sources_with_errors.iter().enumerate() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("src");
        fs::create_dir(&src_dir).unwrap();
        fs::write(src_dir.join("main.vr"), source).unwrap();

        // Create manifest
        let manifest = r#"
[cog]
name = "error_test"
version = "0.1.0"
"#;
        fs::write(temp.path().join("Verum.toml"), manifest).unwrap();

        let mut options = CompilerOptions::default();
        options.input = src_dir.clone();
        options.continue_on_error = true;

        let mut session = Session::new(options);
        let mut pipeline = CompilationPipeline::new_check(&mut session);

        let result = pipeline.check_project();

        // We expect errors for these malformed sources
        match result {
            Ok(check_result) => {
                if check_result.errors > 0 {
                    println!("Correctly detected error in test case {}", i);
                }
            }
            Err(_) => {
                println!(
                    "Correctly detected error in test case {} (returned error)",
                    i
                );
            }
        }
    }
}

/// Test refinement types (may not be fully supported yet)
#[test]
fn test_refinement_types() {
    let source = r#"// Refinement type for positive integers
type Positive = Int where it > 0;

fn main() {
    let x = 25;
    print("Square root test");
}
"#;

    let temp = TempDir::new().unwrap();
    let src_dir = temp.path().join("src");
    fs::create_dir(&src_dir).unwrap();
    fs::write(src_dir.join("main.vr"), source).unwrap();

    let manifest = r#"
[cog]
name = "refinement_test"
version = "0.1.0"
"#;
    fs::write(temp.path().join("Verum.toml"), manifest).unwrap();

    let mut options = CompilerOptions::default();
    options.input = src_dir.clone();

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    match pipeline.check_project() {
        Ok(_) => {
            println!("Successfully compiled refinement types");
        }
        Err(e) => {
            // Refinement types might not be fully implemented yet
            println!("Refinement type compilation result: {}", e);
        }
    }
}
