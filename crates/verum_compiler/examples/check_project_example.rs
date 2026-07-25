//! Example demonstrating the check_project() method
//!
//! This shows how to use the compilation pipeline to type-check
//! a multi-file project without generating any code.

use anyhow::Result;
use std::path::PathBuf;
use tempfile::TempDir;
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

fn main() -> Result<()> {
    // Create a temporary project directory
    let temp_dir = TempDir::new()?;
    println!(
        "Created temporary project at: {}",
        temp_dir.path().display()
    );

    // Create main.vr
    let main_file = temp_dir.path().join("main.vr");
    std::fs::write(
        &main_file,
        r#"
fn factorial(n: Int) -> Int {
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}

fn main() -> Int {
    factorial(5)
}
"#,
    )?;
    println!("Created main.vr");

    // Create utils.vr
    let utils_file = temp_dir.path().join("utils.vr");
    std::fs::write(
        &utils_file,
        r#"
pub fn add(a: Int, b: Int) -> Int {
    a + b
}

pub fn multiply(a: Int, b: Int) -> Int {
    a * b
}

pub fn power(base: Int, exp: Int) -> Int {
    if exp == 0 {
        1
    } else {
        multiply(base, power(base, exp - 1))
    }
}
"#,
    )?;
    println!("Created utils.vr");

    // Create compiler session
    let options = CompilerOptions {
        input: main_file.clone(),
        output: PathBuf::new(),
        verbose: 1,
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new_check(&mut session);

    println!("\nRunning type checking on project...\n");

    // Run check_project
    match pipeline.check_project() {
        Ok(result) => {
            println!("\n{}", "=".repeat(60));
            println!("Check Result:");
            println!("{}", "=".repeat(60));
            println!("{}", result.summary());
            println!("{}", "=".repeat(60));

            if result.is_ok() {
                println!("\n✓ Type checking succeeded!");
            } else {
                println!("\n✗ Type checking failed with {} error(s)", result.errors);
            }
        }
        Err(e) => {
            eprintln!("\n✗ Type checking failed: {}", e);
            return Err(e);
        }
    }

    Ok(())
}
