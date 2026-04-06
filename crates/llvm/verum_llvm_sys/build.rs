//! Build script for verum_llvm_sys
//!
//! This script handles:
//! 1. Using local LLVM installation from llvm/install/ (PRIMARY)
//! 2. Optional override via VERUM_LLVM_DIR environment variable
//! 3. Compiling C/C++ wrappers (target init, LLD)
//! 4. Linking all LLVM, LLD, and MLIR static libraries

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Expected LLVM version (for display/documentation)
const EXPECTED_LLVM_VERSION: &str = "21.0.0";

/// Minimum LLVM major version required
const LLVM_MIN_MAJOR: u32 = 21;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrappers/target.c");
    println!("cargo:rerun-if-changed=src/lld/linker.cpp");
    println!("cargo:rerun-if-env-changed=VERUM_LLVM_DIR");
    println!("cargo:rerun-if-changed=../../../llvm/install/bin/llvm-config");

    // Find LLVM installation - local llvm/install/ is PRIMARY
    let llvm_dir = get_llvm_install_dir();

    // Verify LLVM version
    verify_llvm_version(&llvm_dir);

    // Get LLVM configuration
    let llvm_config = llvm_dir.join("bin/llvm-config");
    if !llvm_config.exists() {
        panic!(
            "llvm-config not found at {}. Run: cd llvm && ./build.sh",
            llvm_config.display()
        );
    }

    // Set include path for downstream crates
    let include_dir = llvm_dir.join("include");
    println!("cargo:include={}", include_dir.display());
    println!("cargo:root={}", llvm_dir.display());

    // Link LLVM libraries
    link_llvm_libraries(&llvm_dir, &llvm_config);

    // Compile C wrappers (target initialization macros)
    compile_target_wrappers(&include_dir);

    // Compile LLD C++ wrapper if feature enabled and LLD headers available
    #[cfg(feature = "lld")]
    {
        let lld_header = llvm_dir.join("include/lld/Common/Driver.h");
        if lld_header.exists() {
            compile_lld_wrapper(&llvm_dir);
        } else {
            println!(
                "cargo:warning=LLD headers not found at {}, skipping LLD wrapper compilation",
                lld_header.display()
            );
            println!("cargo:warning=To enable LLD, build LLVM from source: cd llvm && ./build.sh");
        }
    }

    // Link system libraries
    link_system_libraries();
}

/// Find LLVM installation directory
///
/// Search order:
/// 1. VERUM_LLVM_DIR environment variable (explicit override)
/// 2. Local llvm/install/ directory (PRIMARY - built from source)
///
/// System LLVM is NOT used - we require our own build for consistency
fn get_llvm_install_dir() -> PathBuf {
    // 1. Check explicit environment variable override
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = PathBuf::from(&dir);
        if path.join("bin/llvm-config").exists() {
            return path;
        }
        println!("cargo:warning=VERUM_LLVM_DIR={} but llvm-config not found there", dir);
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

    // Check if build is in progress
    let build_dir = workspace_root.join("llvm/build");
    if build_dir.exists() {
        panic!(
            r#"
LLVM build in progress but not complete!

The llvm/build directory exists but llvm/install is not ready.
Please wait for the build to complete:

  cd llvm && ./build.sh

Or check build progress:
  tail -f llvm/build/build.log (if logging enabled)

Build typically takes 30-60 minutes.
"#
        );
    }

    // No local build found
    panic!(
        r#"
Local LLVM installation not found!

verum_llvm_sys requires a local LLVM build for consistency.
System LLVM (homebrew, apt, etc.) is NOT used.

To build LLVM locally:

  cd llvm && ./build.sh

This will:
  1. Clone llvm-project (if needed)
  2. Build LLVM + LLD + MLIR with static libraries
  3. Install to llvm/install/

Build configuration is in llvm/llvm.toml

Alternatively, set VERUM_LLVM_DIR to override:
  export VERUM_LLVM_DIR=/path/to/custom/llvm

Expected structure:
  llvm/
    install/
      bin/llvm-config
      lib/libLLVM*.a
      lib/libLLD*.a
      lib/libMLIR*.a
      include/
"#
    );
}

/// Verify LLVM version matches expected
fn verify_llvm_version(llvm_dir: &Path) {
    let llvm_config = llvm_dir.join("bin/llvm-config");

    let output = Command::new(&llvm_config)
        .arg("--version")
        .output()
        .expect("Failed to run llvm-config --version");

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Parse major version
    let major: u32 = version
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if major < LLVM_MIN_MAJOR {
        panic!(
            "LLVM version {} is too old. Expected: {}, Minimum required: {}.x\n\
             Please rebuild: cd llvm && ./build.sh",
            version, EXPECTED_LLVM_VERSION, LLVM_MIN_MAJOR
        );
    }
}

/// Link LLVM static libraries
fn link_llvm_libraries(llvm_dir: &Path, llvm_config: &Path) {
    let lib_dir = llvm_dir.join("lib");

    // Add library search path
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Get list of LLVM libraries via llvm-config
    let output = Command::new(llvm_config)
        .args(["--link-static", "--libnames"])
        .output()
        .expect("Failed to run llvm-config --libnames");

    let libs_output = String::from_utf8_lossy(&output.stdout);

    for lib in libs_output.split_whitespace() {
        // Extract library name from filename (libLLVMCore.a -> LLVMCore)
        let lib_name = lib
            .strip_prefix("lib")
            .unwrap_or(lib)
            .strip_suffix(".a")
            .or_else(|| lib.strip_suffix(".lib"))
            .unwrap_or(lib);

        println!("cargo:rustc-link-lib=static={}", lib_name);
    }

    // Link LLD libraries if feature enabled
    #[cfg(feature = "lld")]
    {
        // LLD libraries in dependency order
        let lld_libs = [
            "lldCommon",
            "lldELF",
            "lldMachO",
            "lldCOFF",
            "lldWasm",
            "lldMinGW",
        ];

        for lib in lld_libs {
            let lib_path = lib_dir.join(format!("lib{}.a", lib));
            if lib_path.exists() {
                println!("cargo:rustc-link-lib=static={}", lib);
            }
        }
    }

    // Link MLIR libraries
    link_mlir_libraries(&lib_dir);
}

/// Link MLIR static libraries
fn link_mlir_libraries(lib_dir: &Path) {
    // Scan for MLIR libraries
    if let Ok(entries) = fs::read_dir(lib_dir) {
        let mut mlir_libs: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|entry| {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with("libMLIR") && name_str.ends_with(".a") {
                    Some(
                        name_str
                            .strip_prefix("lib")
                            .unwrap()
                            .strip_suffix(".a")
                            .unwrap()
                            .to_string()
                    )
                } else {
                    None
                }
            })
            .collect();

        // Sort for deterministic linking order
        mlir_libs.sort();

        for lib_name in mlir_libs {
            println!("cargo:rustc-link-lib=static={}", lib_name);
        }
    }
}

/// Compile C wrapper for LLVM target initialization macros
fn compile_target_wrappers(include_dir: &Path) {
    let wrapper_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("wrappers/target.c");

    if !wrapper_path.exists() {
        // Create minimal wrapper if it doesn't exist
        let wrapper_content = r#"
#include "llvm-c/Target.h"

void verum_llvm_initialize_all_targets(void) {
    LLVM_InitializeAllTargets();
}

void verum_llvm_initialize_all_target_infos(void) {
    LLVM_InitializeAllTargetInfos();
}

void verum_llvm_initialize_all_target_mcs(void) {
    LLVM_InitializeAllTargetMCs();
}

void verum_llvm_initialize_all_asm_printers(void) {
    LLVM_InitializeAllAsmPrinters();
}

void verum_llvm_initialize_all_asm_parsers(void) {
    LLVM_InitializeAllAsmParsers();
}

void verum_llvm_initialize_native_target(void) {
    LLVM_InitializeNativeTarget();
}

void verum_llvm_initialize_native_asm_printer(void) {
    LLVM_InitializeNativeAsmPrinter();
}

void verum_llvm_initialize_native_asm_parser(void) {
    LLVM_InitializeNativeAsmParser();
}
"#;
        fs::create_dir_all(wrapper_path.parent().unwrap()).ok();
        fs::write(&wrapper_path, wrapper_content).expect("Failed to create target wrapper");
    }

    cc::Build::new()
        .file(&wrapper_path)
        .include(include_dir)
        .compile("verum_target_wrappers");
}

/// Compile LLD C++ wrapper
#[cfg(feature = "lld")]
fn compile_lld_wrapper(llvm_dir: &Path) {
    let wrapper_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("src/lld/linker.cpp");

    if !wrapper_path.exists() {
        println!("cargo:warning=LLD wrapper not found, skipping: {}", wrapper_path.display());
        return;
    }

    let include_dir = llvm_dir.join("include");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(&wrapper_path)
        .include(&include_dir)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-fno-rtti")
        .flag_if_supported("-fno-exceptions");

    // Platform-specific flags
    #[cfg(target_os = "macos")]
    {
        build.flag("-stdlib=libc++");
    }

    build.compile("verum_lld_wrapper");
}

/// Link system libraries required by LLVM
fn link_system_libraries() {
    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-lib=stdc++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=rt");
    }

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=c++");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
    }

    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=uuid");
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=advapi32");
    }
}
