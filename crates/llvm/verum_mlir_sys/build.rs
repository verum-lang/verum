//! Build script for verum_mlir_sys
//!
//! This script handles:
//! 1. Using local LLVM installation from llvm/install/ (PRIMARY)
//! 2. Optional override via VERUM_LLVM_DIR environment variable
//! 3. Generating bindings for MLIR C API
//! 4. Linking all MLIR static libraries

use std::env;
use std::ffi::OsStr;
use std::fs::read_dir;
use std::path::{Path, PathBuf};
use std::process::Command;

/// LLVM version we expect
const LLVM_MIN_MAJOR: u32 = 21;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=VERUM_LLVM_DIR");
    println!("cargo:rerun-if-changed=../../../llvm/install/bin/llvm-config");

    // Find LLVM installation - local llvm/install/ is PRIMARY
    let llvm_dir = get_llvm_install_dir();

    // Verify LLVM version
    verify_llvm_version(&llvm_dir);

    // Get LLVM configuration (llvm-config on Unix, llvm-config.exe on Windows)
    let llvm_config = if cfg!(windows) {
        let exe_path = llvm_dir.join("bin/llvm-config.exe");
        if exe_path.exists() { exe_path } else { llvm_dir.join("bin/llvm-config") }
    } else {
        llvm_dir.join("bin/llvm-config")
    };
    if !llvm_config.exists() {
        let build_cmd = if cfg!(windows) { r#"cd llvm && .\build.bat"# } else { "cd llvm && ./build.sh" };
        panic!(
            "llvm-config not found at {}. Run: {}",
            llvm_config.display(), build_cmd
        );
    }

    let include_dir = llvm_dir.join("include");
    let lib_dir = llvm_dir.join("lib");

    // Export paths for downstream crates
    println!("cargo:include={}", include_dir.display());
    println!("cargo:root={}", llvm_dir.display());
    println!("cargo:libdir={}", lib_dir.display());

    // Link MLIR libraries
    link_mlir_libraries(&llvm_dir, &llvm_config);

    // Generate bindings
    generate_bindings(&include_dir);
}

/// Check if an LLVM installation directory contains llvm-config.
fn has_llvm_config(dir: &Path) -> bool {
    dir.join("bin/llvm-config").exists() || dir.join("bin/llvm-config.exe").exists()
}

/// Find LLVM installation directory
fn get_llvm_install_dir() -> PathBuf {
    // 1. Check explicit environment variable override
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = PathBuf::from(&dir);
        if has_llvm_config(&path) {
            return path;
        }
        println!(
            "cargo:warning=VERUM_LLVM_DIR={} but llvm-config not found there",
            dir
        );
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

    if has_llvm_config(&local_install) {
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

Build typically takes 30-60 minutes.
"#
        );
    }

    // No local build found
    panic!(
        r#"
Local LLVM installation not found!

verum_mlir_sys requires a local LLVM build for consistency.
System LLVM (homebrew, apt, etc.) is NOT used.

To build LLVM locally:

  cd llvm && ./build.sh

This will:
  1. Clone llvm-project (if needed)
  2. Build LLVM + LLD + MLIR with static libraries
  3. Install to llvm/install/

Alternatively, set VERUM_LLVM_DIR to override:
  export VERUM_LLVM_DIR=/path/to/custom/llvm
"#
    );
}

/// Verify LLVM version matches expected
fn verify_llvm_version(llvm_dir: &Path) {
    let llvm_config = if cfg!(windows) {
        let exe_path = llvm_dir.join("bin/llvm-config.exe");
        if exe_path.exists() { exe_path } else { llvm_dir.join("bin/llvm-config") }
    } else {
        llvm_dir.join("bin/llvm-config")
    };

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
            "LLVM version {} is too old. Minimum required: {}.x\n\
             Please rebuild: cd llvm && ./build.sh",
            version, LLVM_MIN_MAJOR
        );
    }
}

/// Library-name fragments that identify MLIR dialects / helpers Verum
/// never reaches at runtime. Anything whose stem *contains* one of these
/// substrings is excluded from the link line.
///
/// Rationale (Verum uses CPU codegen for X86/AArch64/WebAssembly and GPU
/// codegen via NVVM/ROCDL/SPIRV). The dialects below are specialised
/// extensions that none of the Verum passes target.
const MLIR_LIB_SKIP_FRAGMENTS: &[&str] = &[
    // Framework directives / HPC runtimes not in the Verum pipeline.
    "OpenMP", "OpenACC", "MPI",
    // Tensor Operator Set Architecture — not part of Verum's tensor path.
    "Tosa",
    // Architecture-specific vector extensions Verum does not emit.
    "ArmSME", "ArmSVE", "ArmNeon", "AMX", "X86Vector",
    // Sparse-tensor and friends.
    "SparseTensor",
    // Specialty / research dialects.
    "Quant", "Polynomial", "Mesh", "Shape",
    // Pattern-Descriptor DSL — only needed when user code defines dialects.
    "IRDL", "PDL", "PDLInterp", "PDLToPDLInterp",
    // C-source emission backend.
    "EmitC", "TargetCpp",
    // Async runtime (Verum has its own async in core/async/).
    "AsyncDialect", "AsyncToLLVM", "AsyncToAsyncRuntime", "AsyncTransforms",
    "AsyncRuntime",
    // High-level GPU wrapper (we lower straight to NVVM / ROCDL / SPIRV).
    "NVGPU",
];

/// LLVM component names that ship in `llvm-config --libnames` but are
/// only relevant to host-tooling paths Verum never enters. Each entry is
/// matched against the stripped library name (`LLVMFoo`) as an exact
/// prefix — e.g. `LLVMXRay` matches `LLVMXRay` but not `LLVMCore`.
const LLVM_LIB_SKIP_EXACT: &[&str] = &[
    "LLVMXRay",                        // runtime tracing
    "LLVMCoverage",                    // code-coverage tooling
    "LLVMLineEditor",                  // REPL line editor
    "LLVMLibDriver",                   // lib.exe driver shim
    "LLVMDlltoolDriver",               // Windows dlltool driver
    "LLVMWindowsManifest",             // Windows manifest compiler
    "LLVMTextAPIBinaryReader",         // Apple .tbd reader
    "LLVMTelemetry",                   // telemetry hooks
    "LLVMSymbolize",                   // addr2line / llvm-symbolizer
    "LLVMDebugInfoPDB",                // Windows PDB debuginfo
    "LLVMDebugInfoMSF",                // Windows MSF container
    "LLVMDebugInfoLogicalView",        // specialised debug-info viewer
    "LLVMDebugInfoBTF",                // BPF BTF debug format
    "LLVMDebugInfoGSYM",               // GSYM symbolication format
    "LLVMDWP",                         // DWARF package tool
    "LLVMObjCopy",                     // llvm-objcopy
    "LLVMMCA",                         // machine-code analyser
];

fn mlir_lib_is_skipped(name: &str) -> bool {
    MLIR_LIB_SKIP_FRAGMENTS
        .iter()
        .any(|fragment| name.contains(fragment))
}

fn llvm_lib_is_skipped(stem: &str) -> bool {
    LLVM_LIB_SKIP_EXACT.iter().any(|exact| stem == *exact)
}

/// Link MLIR static libraries
fn link_mlir_libraries(llvm_dir: &Path, llvm_config: &Path) {
    let lib_dir = llvm_dir.join("lib");

    // Add library search path
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Link MLIR static libraries, skipping dialects that Verum does not
    // reach. See `MLIR_LIB_SKIP_FRAGMENTS` for rationale.
    if let Ok(entries) = read_dir(&lib_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if !name.starts_with("libMLIR") || !name.ends_with(".a") {
                continue;
            }
            let Some(lib_name) = parse_archive_name(name) else {
                continue;
            };
            if mlir_lib_is_skipped(lib_name) {
                continue;
            }
            println!("cargo:rustc-link-lib=static={}", lib_name);
        }
    }

    // Get LLVM libraries via llvm-config, filtering host-tooling-only
    // components that never land on the Verum runtime code path.
    let output = Command::new(llvm_config)
        .args(["--link-static", "--libnames"])
        .output()
        .expect("Failed to run llvm-config --libnames");

    let libs_output = String::from_utf8_lossy(&output.stdout);
    for name in libs_output.trim().split(' ') {
        let Some(lib_name) = parse_archive_name(name) else {
            continue;
        };
        if llvm_lib_is_skipped(lib_name) {
            continue;
        }
        println!("cargo:rustc-link-lib={}", lib_name);
    }

    // Link system libraries
    let output = Command::new(llvm_config)
        .args(["--link-static", "--system-libs"])
        .output()
        .expect("Failed to run llvm-config --system-libs");

    let system_libs = String::from_utf8_lossy(&output.stdout);
    for flag in system_libs.trim().split(' ') {
        let flag = flag.trim_start_matches("-l");
        if flag.is_empty() {
            continue;
        }

        if flag.starts_with('/') {
            // Absolute path to dynamic library
            let path = Path::new(flag);
            if let Some(parent) = path.parent() {
                println!("cargo:rustc-link-search={}", parent.display());
            }
            if let Some(stem) = path.file_stem().and_then(OsStr::to_str) {
                println!("cargo:rustc-link-lib={}", stem.trim_start_matches("lib"));
            }
        } else {
            // Strip .lib/.a suffix — Rust's linker appends the platform extension.
            let lib_name = flag
                .strip_suffix(".lib")
                .or_else(|| flag.strip_suffix(".a"))
                .unwrap_or(flag);
            println!("cargo:rustc-link-lib={}", lib_name);
        }
    }

    // Link C++ standard library
    if let Some(name) = get_system_libcpp() {
        println!("cargo:rustc-link-lib={}", name);
    }
}

/// Generate Rust bindings for MLIR C API
fn generate_bindings(include_dir: &Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let wrapper_path = manifest_dir.join("wrapper.h");

    bindgen::builder()
        .header(wrapper_path.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Failed to generate MLIR bindings")
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Failed to write bindings");
}

fn get_system_libcpp() -> Option<&'static str> {
    if cfg!(target_env = "msvc") {
        None
    } else if cfg!(target_os = "macos") {
        Some("c++")
    } else {
        Some("stdc++")
    }
}

fn parse_archive_name(name: &str) -> Option<&str> {
    name.strip_prefix("lib")
        .and_then(|n| n.strip_suffix(".a"))
}
