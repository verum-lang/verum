#![allow(clippy::disallowed_types)] // Build scripts can use String/Vec directly

//! Build script for verum_cli.
//!
//! Sets up environment variables for the CLI.

use std::env;

fn main() {
    // Export TARGET for runtime detection
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=VERUM_HOST_TARGET={}", target);
    println!("cargo:rustc-env=TARGET={}", target);

    // Note: LLVM LTO symbols are handled by the stub module on MSVC
    // (see verum_llvm/src/lib.rs). No /WHOLEARCHIVE needed.

    println!("cargo:rerun-if-changed=build.rs");
}

/// Find the LLVM install directory (same logic as verum_llvm_sys)
fn find_llvm_install_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = std::path::PathBuf::from(&dir);
        if path.join("lib").exists() {
            return Some(path);
        }
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let workspace_root = std::path::PathBuf::from(&manifest_dir)
        .parent()? // crates/
        .parent()? // verum/
        .to_path_buf();
    let local_install = workspace_root.join("llvm/install");
    if local_install.join("lib").exists() {
        Some(local_install)
    } else {
        None
    }
}
