//! Build script for verum_mlir_macro
//!
//! Sets the LLVM_INCLUDE_DIRECTORY environment variable for the macro crate
//! to find MLIR TableGen files.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-env-changed=VERUM_LLVM_DIR");
    println!("cargo:rerun-if-changed=../../../llvm/install/bin/llvm-config");

    // Find LLVM installation
    let llvm_dir = get_llvm_install_dir();
    let llvm_config = llvm_dir.join("bin/llvm-config");

    // Get include directory
    let include_dir = run_llvm_config(&llvm_config, "--includedir");

    // Export to the macro for finding TableGen files
    println!("cargo:rustc-env=LLVM_INCLUDE_DIRECTORY={}", include_dir);
}

/// Find LLVM installation directory
fn get_llvm_install_dir() -> PathBuf {
    // 1. Check explicit environment variable override
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = PathBuf::from(&dir);
        if path.join("bin/llvm-config").exists() {
            return path;
        }
    }

    // 2. Use local llvm/install/ directory (PRIMARY)
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = PathBuf::from(&manifest_dir)
        .parent() // crates/llvm/
        .and_then(|p| p.parent()) // crates/
        .and_then(|p| p.parent()) // axiom/
        .unwrap()
        .to_path_buf();

    let local_install = workspace_root.join("llvm/install");

    if local_install.join("bin/llvm-config").exists() {
        return local_install;
    }

    panic!(
        r#"
Local LLVM installation not found!

verum_mlir_macro requires a local LLVM build.

To build LLVM locally:

  cd llvm && ./build.sh

Alternatively, set VERUM_LLVM_DIR to override:
  export VERUM_LLVM_DIR=/path/to/custom/llvm
"#
    );
}

fn run_llvm_config(llvm_config: &Path, arg: &str) -> String {
    let output = Command::new(llvm_config)
        .args(["--link-static", arg])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run llvm-config {}: {}", arg, e));

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
