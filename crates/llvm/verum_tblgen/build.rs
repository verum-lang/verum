//! Build script for verum_tblgen
//!
//! This script handles:
//! 1. Using local LLVM installation from llvm/install/ (PRIMARY)
//! 2. Optional override via VERUM_LLVM_DIR environment variable
//! 3. Compiling TableGen C++ wrappers
//! 4. Generating Rust bindings for TableGen

use std::{
    env,
    ffi::OsStr,
    fs::read_dir,
    path::{Path, PathBuf},
    process::Command,
};

/// Minimum LLVM major version required
const LLVM_MIN_MAJOR: u32 = 21;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=cc");
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

    let include_dir = llvm_dir.join("include");
    let lib_dir = llvm_dir.join("lib");

    // Export paths for downstream crates
    println!("cargo:include={}", include_dir.display());
    println!("cargo:root={}", llvm_dir.display());
    println!("cargo:libdir={}", lib_dir.display());

    // Add library search path
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    // Build TableGen C++ library
    build_c_library(&llvm_config, &include_dir);

    // Link LLVM libraries
    link_llvm_libraries(&llvm_config);

    // Generate bindings
    generate_bindings(&include_dir);
}

/// Find LLVM installation directory
fn get_llvm_install_dir() -> PathBuf {
    // 1. Check explicit environment variable override
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = PathBuf::from(&dir);
        if path.join("bin/llvm-config").exists() {
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

Build typically takes 30-60 minutes.
"#
        );
    }

    // No local build found
    panic!(
        r#"
Local LLVM installation not found!

verum_tblgen requires a local LLVM build for consistency.
System LLVM (homebrew, apt, etc.) is NOT used.

To build LLVM locally:

  cd llvm && ./build.sh

Alternatively, set VERUM_LLVM_DIR to override:
  export VERUM_LLVM_DIR=/path/to/custom/llvm
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
            "LLVM version {} is too old. Minimum required: {}.x\n\
             Please rebuild: cd llvm && ./build.sh",
            version, LLVM_MIN_MAJOR
        );
    }
}

/// Build the TableGen C++ wrapper library
fn build_c_library(llvm_config: &Path, include_dir: &Path) {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Get LLVM compiler flags
    let cxxflags = run_llvm_config(llvm_config, "--cxxflags");
    let cflags = run_llvm_config(llvm_config, "--cflags");

    unsafe {
        env::set_var("CXXFLAGS", &cxxflags);
        env::set_var("CFLAGS", &cflags);
    }

    // Find all C++ source files
    let lib_dir = manifest_dir.join("cc/lib");
    let cpp_files: Vec<PathBuf> = read_dir(&lib_dir)
        .expect("Failed to read cc/lib directory")
        .filter_map(|e| e.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension() == Some(OsStr::new("cpp")))
        .collect();

    cc::Build::new()
        .cpp(true)
        .files(cpp_files)
        .include(manifest_dir.join("cc/include"))
        .include(include_dir)
        .flag(if cfg!(target_env = "msvc") {
            "/WX"
        } else {
            "-Werror"
        })
        .std("c++17")
        .compile("CTableGen");
}

/// Link LLVM libraries
fn link_llvm_libraries(llvm_config: &Path) {
    // Get LLVM library names
    let libnames = run_llvm_config(llvm_config, "--libnames");
    for name in libnames.trim().split(' ') {
        if let Some(lib_name) = parse_library_name(name) {
            println!("cargo:rustc-link-lib=static={}", lib_name);
        }
    }

    // Link system libraries
    let system_libs = run_llvm_config(llvm_config, "--system-libs");
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
            if let Some(lib_name) = path
                .file_name()
                .and_then(OsStr::to_str)
                .and_then(parse_library_name)
            {
                println!("cargo:rustc-link-lib={}", lib_name);
            }
        } else {
            println!("cargo:rustc-link-lib={}", flag);
        }
    }

    // Link C++ standard library
    if let Some(name) = get_system_libcpp() {
        println!("cargo:rustc-link-lib={}", name);
    }
}

/// Generate Rust bindings for TableGen
fn generate_bindings(include_dir: &Path) {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    bindgen::builder()
        .header(manifest_dir.join("wrapper.h").to_string_lossy())
        .clang_arg(format!("-I{}", manifest_dir.join("cc/include").display()))
        .clang_arg(format!("-I{}", include_dir.display()))
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Failed to generate TableGen bindings")
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Failed to write bindings");
}

fn run_llvm_config(llvm_config: &Path, arg: &str) -> String {
    let output = Command::new(llvm_config)
        .args(["--link-static", arg])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run llvm-config {}: {}", arg, e));

    String::from_utf8_lossy(&output.stdout).trim().to_string()
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

fn parse_library_name(name: &str) -> Option<&str> {
    name.strip_prefix("lib")
        .and_then(|name| name.split('.').next())
}
