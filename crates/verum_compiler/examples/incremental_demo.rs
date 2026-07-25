//! Demonstration of incremental compilation
//!
//! This example shows how the incremental compiler tracks changes
//! and minimizes recompilation work.
//!
//! Run with:
//! ```bash
//! cargo run --example incremental_demo
//! ```

use std::fs;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use verum_ast::{FileId, Module};
use verum_compiler::IncrementalCompiler;

fn create_test_module(file_id: u32) -> Module {
    Module::empty(FileId::new(file_id))
}

fn main() {
    println!("=== Verum Incremental Compilation Demo ===\n");

    // Create temporary project
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let project_root = temp_dir.path().to_path_buf();

    println!("Project root: {}\n", project_root.display());

    // Create source files
    let main_file = project_root.join("main.vr");
    let lib_file = project_root.join("lib.vr");
    let utils_file = project_root.join("utils.vr");

    fs::write(&main_file, "import lib;\nfn main() { lib_func(); }")
        .expect("Failed to write main.vr");
    fs::write(&lib_file, "import utils;\nfn lib_func() { util(); }")
        .expect("Failed to write lib.vr");
    fs::write(&utils_file, "fn util() { println!(\"Hello\"); }").expect("Failed to write utils.vr");

    println!("Created source files:");
    println!("  - main.vr (imports lib)");
    println!("  - lib.vr (imports utils)");
    println!("  - utils.vr\n");

    // === First Compilation ===
    println!("--- First Compilation (cold start) ---");

    let mut compiler = IncrementalCompiler::new();

    let start = Instant::now();

    // Compile all modules
    compiler.cache_module(utils_file.clone(), create_test_module(0));
    compiler.cache_module(lib_file.clone(), create_test_module(1));
    compiler.cache_module(main_file.clone(), create_test_module(2));

    let elapsed = start.elapsed();

    println!("Compiled 3 modules in {:?}", elapsed);
    let stats = compiler.stats();
    println!("Cached modules: {}", stats.cached_modules);
    println!("Meta registry valid: {}\n", stats.meta_registry_valid);

    // === Second Compilation (no changes) ===
    println!("--- Second Compilation (no changes) ---");

    let start = Instant::now();

    let files_to_check = vec![&main_file, &lib_file, &utils_file];
    let mut needs_rebuild = 0;

    for file in &files_to_check {
        if compiler.needs_recompile(file) {
            needs_rebuild += 1;
            println!(
                "  ❌ {} needs recompilation",
                file.file_name().unwrap().to_str().unwrap()
            );
        } else {
            println!(
                "  ✅ {} up to date",
                file.file_name().unwrap().to_str().unwrap()
            );
        }
    }

    let elapsed = start.elapsed();

    println!("\nNeeds rebuild: {}/3 modules", needs_rebuild);
    println!("Check completed in {:?}\n", elapsed);

    // === Third Compilation (utils.vr changed) ===
    println!("--- Third Compilation (utils.vr modified) ---");

    // Wait a bit to ensure different modification time
    thread::sleep(Duration::from_millis(50));

    // Modify utils.vr
    fs::write(&utils_file, "fn util() { println!(\"Hello, World!\"); }")
        .expect("Failed to write utils.vr");

    println!("Modified: utils.vr\n");

    // Check which files now need recompilation
    let mut files_needing_recompile = Vec::new();
    for file in &files_to_check {
        if compiler.needs_recompile(file) {
            files_needing_recompile.push((*file).clone());
        }
    }

    println!("Files needing recompilation:");
    for (i, file) in files_needing_recompile.iter().enumerate() {
        println!(
            "  {}. {}",
            i + 1,
            file.file_name().unwrap().to_str().unwrap()
        );
    }

    println!(
        "\nRecompiling {} of 3 modules ({:.0}% saved)",
        files_needing_recompile.len(),
        (1.0 - files_needing_recompile.len() as f64 / 3.0) * 100.0
    );

    // === Invalidation Demo ===
    println!("\n--- Cache Invalidation Demo ---");

    compiler.invalidate(&utils_file);

    println!("Invalidated: utils.vr and all dependents");
    let stats = compiler.stats();
    println!(
        "Cached modules after invalidation: {}\n",
        stats.cached_modules
    );

    // === Final Stats ===
    println!("--- Final Statistics ---");

    let stats = compiler.stats();
    println!("Cached modules: {}", stats.cached_modules);
    println!("Meta registry valid: {}", stats.meta_registry_valid);

    println!("\n=== Demo Complete ===");

    // Cleanup
    temp_dir.close().ok();
}
