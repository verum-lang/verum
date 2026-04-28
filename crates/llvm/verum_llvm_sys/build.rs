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

    // Get LLVM configuration (llvm-config on Unix, llvm-config.exe on Windows)
    let llvm_config = if cfg!(windows) {
        let exe_path = llvm_dir.join("bin/llvm-config.exe");
        if exe_path.exists() {
            exe_path
        } else {
            llvm_dir.join("bin/llvm-config")
        }
    } else {
        llvm_dir.join("bin/llvm-config")
    };
    if !llvm_config.exists() {
        let build_cmd = if cfg!(windows) {
            r#"cd llvm && .\build.bat"#
        } else {
            "cd llvm && ./build.sh"
        };
        panic!(
            "llvm-config not found at {}. Run: {}",
            llvm_config.display(), build_cmd
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

/// Check if an LLVM installation directory contains llvm-config.
fn has_llvm_config(dir: &Path) -> bool {
    dir.join("bin/llvm-config").exists() || dir.join("bin/llvm-config.exe").exists()
}

/// Find LLVM installation directory
///
/// Search order:
/// 1. VERUM_LLVM_DIR environment variable (explicit override)
/// 2. Local llvm/install/ directory — built automatically via
///    `llvm/build.sh` if missing.
///
/// System LLVM is NOT used — we require our own build for
/// consistency. Auto-invocation of `llvm/build.sh` keeps
/// `cargo build` self-contained: a fresh checkout that lacks
/// `llvm/install/` triggers the source build (~30–60 min) and
/// then continues, instead of stopping the user with manual
/// instructions.
fn get_llvm_install_dir() -> PathBuf {
    // 1. Check explicit environment variable override
    if let Ok(dir) = env::var("VERUM_LLVM_DIR") {
        let path = PathBuf::from(&dir);
        if has_llvm_config(&path) {
            return path;
        }
        println!("cargo:warning=VERUM_LLVM_DIR={} but llvm-config not found there", dir);
    }

    // 2. Use local llvm/install/ directory
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = PathBuf::from(&manifest_dir)
        .parent() // crates/llvm/
        .and_then(|p| p.parent()) // crates/
        .and_then(|p| p.parent()) // workspace root
        .unwrap()
        .to_path_buf();

    let local_install = workspace_root.join("llvm/install");

    if has_llvm_config(&local_install) {
        return local_install;
    }

    // 3. Auto-invoke llvm/build.sh — clones llvm-project, configures
    //    cmake with llvm/llvm.toml, builds LLVM/LLD/MLIR static libs
    //    and installs into llvm/install/.
    auto_build_llvm(&workspace_root);

    // 4. Re-check after the build script ran.
    if has_llvm_config(&local_install) {
        return local_install;
    }

    panic!(
        "LLVM auto-build did not produce {}/bin/llvm-config — \
         inspect llvm/build.log for the failure.",
        local_install.display()
    );
}

/// Run `llvm/build.sh` to populate `llvm/install/`. Aborts the build
/// with a helpful message on platforms that don't ship a POSIX shell
/// (Windows native cargo runs without bash on `$PATH`).
fn auto_build_llvm(workspace_root: &Path) {
    let build_script = workspace_root.join("llvm/build.sh");
    if !build_script.exists() {
        panic!(
            "llvm/build.sh not found at {}. The repository is incomplete.",
            build_script.display()
        );
    }

    println!(
        "cargo:warning=llvm/install/ not found — invoking llvm/build.sh \
         (this clones llvm-project and builds LLVM 21 + LLD + MLIR; \
         expect 30–60 min on a fresh checkout)."
    );

    // On Unix run the script directly; on Windows route through `bash`
    // so Git-Bash / WSL / MSYS2 environments work. Native cmd.exe
    // builds are not supported — set VERUM_LLVM_DIR to a prebuilt LLVM
    // tree instead.
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("bash");
        c.arg(build_script.to_string_lossy().as_ref());
        c
    } else {
        Command::new(&build_script)
    };

    let status = cmd
        .current_dir(workspace_root.join("llvm"))
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "Failed to launch llvm/build.sh: {err}. \
                 Ensure bash + cmake + ninja + a C++ compiler are on \
                 $PATH, or set VERUM_LLVM_DIR to a prebuilt LLVM tree."
            )
        });

    if !status.success() {
        panic!(
            "llvm/build.sh exited with {:?}. \
             See llvm/build.log for details. Common causes: \
             missing cmake/ninja/C++ toolchain, insufficient disk \
             (~50 GB needed), insufficient RAM (~16 GB recommended).",
            status.code()
        );
    }
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
        // Extract library name from filename:
        //   Unix:    libLLVMCore.a   -> LLVMCore
        //   Windows: LLVMCore.lib    -> LLVMCore
        let lib_name = lib
            .strip_prefix("lib")
            .unwrap_or(lib)
            .strip_suffix(".a")
            .or_else(|| lib.strip_suffix(".lib"))
            .unwrap_or(lib);

        if lib_name.is_empty() { continue; }
        println!("cargo:rustc-link-lib=static={}", lib_name);
    }

    // LTO C API library (separate from LLVM component libraries).
    // Contains lto_codegen_*, thinlto_* functions used by verum_llvm::lto.
    // Not included in llvm-config --libnames.
    let lto_lib = lib_dir.join(if cfg!(windows) { "LTO.lib" } else { "libLTO.a" });
    if lto_lib.exists() {
        println!("cargo:rustc-link-lib=static=LTO");
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
            // Check both Unix (lib*.a) and Windows (*.lib) naming
            let unix_path = lib_dir.join(format!("lib{}.a", lib));
            let win_path = lib_dir.join(format!("{}.lib", lib));
            if unix_path.exists() || win_path.exists() {
                println!("cargo:rustc-link-lib=static={}", lib);
            }
        }
    }

    // Link MLIR libraries
    link_mlir_libraries(&lib_dir);
}

/// Link MLIR static libraries
fn link_mlir_libraries(lib_dir: &Path) {
    // Scan for MLIR libraries (lib*.a on Unix, *.lib on Windows)
    if let Ok(entries) = fs::read_dir(lib_dir) {
        let mut mlir_libs: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|entry| {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                // Unix: libMLIR*.a
                if name_str.starts_with("libMLIR") && name_str.ends_with(".a") {
                    Some(
                        name_str
                            .strip_prefix("lib")
                            .unwrap()
                            .strip_suffix(".a")
                            .unwrap()
                            .to_string()
                    )
                // Windows: MLIR*.lib
                } else if name_str.starts_with("MLIR") && name_str.ends_with(".lib") {
                    Some(
                        name_str
                            .strip_suffix(".lib")
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
        .include(&include_dir);

    // Platform-specific C++ flags
    #[cfg(target_os = "windows")]
    {
        build
            .flag_if_supported("/std:c++17")
            .flag_if_supported("/GR-")   // Disable RTTI
            .flag_if_supported("/EHs-c-"); // Disable exceptions
    }

    #[cfg(not(target_os = "windows"))]
    {
        build
            .flag_if_supported("-std=c++17")
            .flag_if_supported("-fno-rtti")
            .flag_if_supported("-fno-exceptions");
    }

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
