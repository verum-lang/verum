#![allow(dead_code, unused_imports)]
//! Build script for `cvc5-sys`.
//!
//! This script provides three modes, selected via feature flags:
//!
//! 1. **`vendored` (default for distribution)**: Build CVC5 from source using
//!    the vendored source tree in `cvc5/`. Produces a static library linked
//!    directly into the final binary. Requires CMake + C++17 compiler + GMP.
//!
//! 2. **`system`**: Link against a system-installed `libcvc5.so`/`libcvc5.dylib`.
//!    Fast builds, but requires the user to install CVC5 separately.
//!
//! 3. **No features (fallback)**: Check `CVC5_ROOT` environment variable for
//!    a pre-built CVC5 installation. If found, use it; otherwise, report an
//!    actionable error telling the user how to proceed.
//!
//! ## Environment Variables
//!
//! * `CVC5_ROOT` — Path to a CVC5 installation (contains `include/` and `lib/`).
//!   Takes precedence over vendored and system builds.
//! * `CVC5_NO_VENDOR` — If set, disables vendored build (useful for CI).
//! * `CVC5_JOBS` — Number of parallel jobs for CMake build (default: `num_cpus`).
//! * `DOCS_RS` — Set by docs.rs; skips linking and provides stub bindings.
//!
//! ## Output
//!
//! Sets the following Cargo instructions:
//! - `cargo:rustc-link-lib=static=cvc5` (and dependencies)
//! - `cargo:rustc-link-search=native=<build_dir>/lib`
//! - `cargo:include=<install_dir>/include`
//! - `cargo:rerun-if-changed=build.rs`
//! - `cargo:rerun-if-env-changed=CVC5_ROOT`

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Minimum supported CVC5 version — bumped when we require new features.
const MIN_CVC5_VERSION: &str = "1.3.0";

/// Default CVC5 version to fetch when vendored source is absent.
const DEFAULT_CVC5_VERSION: &str = "1.3.3";

fn main() {
    // Always rerun when these change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CVC5_ROOT");
    println!("cargo:rerun-if-env-changed=CVC5_NO_VENDOR");
    println!("cargo:rerun-if-env-changed=CVC5_JOBS");
    println!("cargo:rerun-if-env-changed=CVC5_UNSAFE_MODE");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    // Declare known custom cfg flags for check-cfg lint.
    println!("cargo::rustc-check-cfg=cfg(docsrs)");

    // docs.rs: skip actual linking, provide stub bindings only.
    if env::var("DOCS_RS").is_ok() {
        println!("cargo:warning=cvc5-sys: DOCS_RS detected, skipping link");
        return;
    }

    // 1. Prefer explicit CVC5_ROOT if set.
    if let Ok(root) = env::var("CVC5_ROOT") {
        if link_prebuilt(Path::new(&root)).is_ok() {
            return;
        } else {
            panic!(
                "cvc5-sys: CVC5_ROOT={} was set but linking failed. \
                 Ensure {}/lib contains libcvc5.a or libcvc5.dylib/so.",
                root, root
            );
        }
    }

    // 2. System linking mode (feature `system`).
    #[cfg(feature = "system")]
    {
        if link_system().is_ok() {
            return;
        } else {
            panic!(
                "cvc5-sys: feature `system` enabled but CVC5 not found. \
                 Install via `brew install cvc5` (macOS) or `apt install libcvc5-dev` (Linux)."
            );
        }
    }

    // 3. Vendored build mode (feature `vendored` or `static`).
    #[cfg(any(feature = "vendored", feature = "static"))]
    {
        if env::var("CVC5_NO_VENDOR").is_ok() {
            panic!(
                "cvc5-sys: CVC5_NO_VENDOR is set but no CVC5_ROOT provided. \
                 Either unset CVC5_NO_VENDOR or provide CVC5_ROOT=/path/to/cvc5."
            );
        }
        build_vendored();
        return;
    }

    // 4. Fallback: no features, no env var. Emit a clear error.
    #[cfg(not(any(feature = "system", feature = "vendored", feature = "static")))]
    {
        println!(
            "cargo:warning=cvc5-sys: no linking mode configured. \
             Enable feature `vendored` to build CVC5 from source, \
             `system` to link system library, or set CVC5_ROOT environment variable."
        );
        // Emit minimal stub configuration so downstream crates can still build
        // (Cvc5Backend will return NotAvailable at runtime).
        println!("cargo:rustc-cfg=cvc5_stub");
    }
}

/// Link against a pre-built CVC5 installation at `root`.
///
/// Expects the standard layout:
/// ```text
/// <root>/
/// ├── include/cvc5/       — headers
/// ├── lib/libcvc5.{a,dylib,so}
/// └── lib/libcvc5parser.{a,dylib,so}
/// ```
fn link_prebuilt(root: &Path) -> Result<(), String> {
    let lib_dir = root.join("lib");
    let include_dir = root.join("include");

    if !lib_dir.exists() {
        return Err(format!("lib directory not found: {}", lib_dir.display()));
    }
    if !include_dir.exists() {
        return Err(format!("include directory not found: {}", include_dir.display()));
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:include={}", include_dir.display());

    // Determine static vs dynamic based on available files.
    let static_lib = lib_dir.join(lib_name("cvc5", true));
    let dynamic_lib = lib_dir.join(lib_name("cvc5", false));

    if static_lib.exists() {
        link_static_deps();
        println!("cargo:rustc-link-lib=static=cvc5");
    } else if dynamic_lib.exists() {
        println!("cargo:rustc-link-lib=dylib=cvc5");
        // C++ runtime is still needed for exception/RTTI.
        link_cxx_runtime();
    } else {
        return Err(format!(
            "neither static nor dynamic libcvc5 found in {}",
            lib_dir.display()
        ));
    }

    // Parser library is optional but usually present.
    if lib_dir.join(lib_name("cvc5parser", true)).exists() {
        println!("cargo:rustc-link-lib=static=cvc5parser");
    } else if lib_dir.join(lib_name("cvc5parser", false)).exists() {
        println!("cargo:rustc-link-lib=dylib=cvc5parser");
    }

    Ok(())
}

/// Link against system-installed CVC5 via `pkg-config` or standard paths.
#[cfg(feature = "system")]
fn link_system() -> Result<(), String> {
    // Try pkg-config first.
    if let Ok(lib) = pkg_config::Config::new()
        .atleast_version(MIN_CVC5_VERSION)
        .probe("cvc5")
    {
        println!("cargo:include={}", lib.include_paths[0].display());
        return Ok(());
    }

    // Fallback: check standard locations.
    let candidates = [
        "/usr/local",    // Homebrew (Intel Mac), manual install
        "/opt/homebrew", // Homebrew (Apple Silicon)
        "/usr",          // System package
    ];

    for candidate in &candidates {
        if Path::new(candidate).join("include/cvc5/cvc5.h").exists() {
            return link_prebuilt(Path::new(candidate));
        }
    }

    Err("CVC5 not found in system paths".to_string())
}

#[cfg(not(feature = "system"))]
fn link_system() -> Result<(), String> {
    Err("feature `system` not enabled".to_string())
}

/// Build CVC5 from vendored source and statically link it.
///
/// This is the preferred mode for distribution: it produces a self-contained
/// binary with zero runtime dependencies on external SMT solvers.
#[cfg(any(feature = "vendored", feature = "static"))]
fn build_vendored() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Accept either `cvc/` (current submodule path) or `cvc5/` (legacy).
    // We check which exists and use that path.
    let cvc5_src = [manifest_dir.join("cvc"), manifest_dir.join("cvc5")]
        .into_iter()
        .find(|p| p.exists() && p.join("CMakeLists.txt").exists())
        .unwrap_or_else(|| {
            panic!(
                "cvc5-sys: vendored CVC5 source not found at {} or {}.\n\
                 \n\
                 To fix:\n\
                 1. Initialize the submodule:\n\
                      cd {}\n\
                      git submodule update --init --recursive crates/cvc5-sys/cvc\n\
                      cd crates/cvc5-sys/cvc && git checkout cvc5-{}\n\
                 2. Or set CVC5_ROOT to point to a pre-built CVC5 installation.\n\
                 3. Or enable feature `system` to link against a system CVC5.",
                manifest_dir.join("cvc").display(),
                manifest_dir.join("cvc5").display(),
                manifest_dir.parent().unwrap().parent().unwrap().display(),
                DEFAULT_CVC5_VERSION
            );
        });
    println!("cargo:warning=cvc5-sys: using CVC5 source at {}", cvc5_src.display());

    let jobs = env::var("CVC5_JOBS")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| num_logical_cpus());

    let profile = match env::var("PROFILE").as_deref() {
        Ok("release") => "Release",
        _ => "RelWithDebInfo", // Keep debug info in dev builds for profiling.
    };

    println!(
        "cargo:warning=cvc5-sys: building CVC5 from source \
         (profile={}, jobs={}). First build takes 3-5 minutes.",
        profile, jobs
    );

    let mut config = cmake::Config::new(&cvc5_src);
    config
        .profile(profile)
        // === Core build settings ===
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("BUILD_SHARED_LIBS", "OFF")            // CRITICAL: static libraries
        .define("CMAKE_CXX_STANDARD", "17")
        // === Disable language bindings we don't need ===
        .define("BUILD_BINDINGS_PYTHON", "OFF")
        .define("BUILD_BINDINGS_JAVA", "OFF")
        // === Enable C API (required by our FFI bindings) ===
        .define("BUILD_BINDINGS_C", "ON")
        // === Skip tests and benchmarks (faster builds) ===
        .define("ENABLE_UNIT_TESTING", "OFF")
        .define("ENABLE_SYSTEM_TESTS", "OFF")
        // === Auto-download small dependencies (CaDiCaL, ANTLR, SymFPU) ===
        .define("ENABLE_AUTO_DOWNLOAD", "ON")
        // === Safety mode (CVC5 1.3.0+) ===
        // `safe-mode=safe` guards all CVC5 features that are either not robust
        // or lack full proof/model support. We enable it by default for
        // production use; relax via `CVC5_UNSAFE_MODE=1` for experimental features.
        .define(
            "CVC5_SAFE_MODE",
            if env::var("CVC5_UNSAFE_MODE").is_ok() { "none" } else { "safe" }
        )
        // === SMT theory extensions ===
        .define("USE_POLY", "ON")                      // LibPoly: polynomial arithmetic (NRA)
        // === License-compatible dependencies only (no GPL) ===
        .define("USE_CLN", feature_gpl("ON", "OFF"))   // CLN is GPL — off by default
        .define("USE_CRYPTOMINISAT", feature_gpl("ON", "OFF")) // GPL — off
        // === Optimizations ===
        .define("ENABLE_ASSERTIONS", bool_define(profile != "Release"))
        .define("ENABLE_VALGRIND", "OFF")
        .define("ENABLE_COVERAGE", "OFF");

    // Parallel jobs for the underlying build system (make/ninja).
    config.env("CMAKE_BUILD_PARALLEL_LEVEL", jobs.to_string());

    let install_dir = config.build();

    // === Emit link instructions ===
    let lib_dir = install_dir.join("lib");
    let include_dir = install_dir.join("include");

    if !lib_dir.exists() {
        panic!(
            "cvc5-sys: CMake build completed but lib directory missing: {}",
            lib_dir.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:include={}", include_dir.display());
    println!("cargo:root={}", install_dir.display());

    // Primary CVC5 libraries (order matters for static linking).
    println!("cargo:rustc-link-lib=static=cvc5parser");
    println!("cargo:rustc-link-lib=static=cvc5");

    // Link CVC5's bundled transitive dependencies.
    link_static_deps();

    // C++ runtime (required for exception handling, RTTI, STL).
    link_cxx_runtime();
}

#[cfg(not(any(feature = "vendored", feature = "static")))]
fn build_vendored() {
    unreachable!("build_vendored() called without vendored/static feature");
}

/// Link CVC5's statically-bundled dependencies.
///
/// CVC5 ships with several dependencies that must be linked in the correct order:
/// - `cadical`: SAT solver (BSD)
/// - `antlr4-runtime`: Parser runtime (BSD)
/// - `poly`/`polyxx`: Polynomial arithmetic (BSD)
/// - `gmp`/`gmpxx`: Arbitrary precision arithmetic (LGPL — dynamically linked by default)
/// - `symfpu`: Floating-point bit-blasting (MIT)
fn link_static_deps() {
    // Order: depended-upon libraries last (reverse topological order).
    for lib in &[
        "cadical",
        "antlr4-runtime",
        "polyxx",
        "poly",
        "symfpu",
    ] {
        println!("cargo:rustc-link-lib=static={}", lib);
    }

    // GMP is linked dynamically by default (LGPL).
    // To statically link, the user must provide libgmp.a via GMP_STATIC_LIB.
    if let Ok(gmp_static) = env::var("GMP_STATIC_LIB") {
        println!("cargo:rustc-link-search=native={}", gmp_static);
        println!("cargo:rustc-link-lib=static=gmp");
        println!("cargo:rustc-link-lib=static=gmpxx");
    } else {
        println!("cargo:rustc-link-lib=dylib=gmp");
        println!("cargo:rustc-link-lib=dylib=gmpxx");
    }
}

/// Link the C++ standard library required by CVC5.
///
/// Platform-specific:
/// - macOS: `libc++` (Clang)
/// - Linux: `libstdc++` (GCC) or `libc++` (Clang via CXX=clang++)
/// - Windows: MSVC runtime (handled automatically)
fn link_cxx_runtime() {
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=dylib=c++");
    }
    #[cfg(target_os = "linux")]
    {
        // Detect compiler: prefer libstdc++ for GCC, libc++ for Clang.
        let cxx = env::var("CXX").unwrap_or_else(|_| "g++".to_string());
        if cxx.contains("clang") {
            println!("cargo:rustc-link-lib=dylib=c++");
            println!("cargo:rustc-link-lib=dylib=c++abi");
        } else {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
    }
    #[cfg(target_os = "windows")]
    {
        // MSVC automatically links the C++ runtime. No explicit link needed.
    }
}

/// Compute a library filename for the target platform.
fn lib_name(base: &str, static_lib: bool) -> String {
    let prefix = if cfg!(target_os = "windows") { "" } else { "lib" };
    let ext = if static_lib {
        if cfg!(target_os = "windows") { "lib" } else { "a" }
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    format!("{}{}.{}", prefix, base, ext)
}

/// Return `on` if feature `gpl` is enabled, else `off`.
///
/// Used to toggle GPL-licensed dependencies (CLN, CryptoMiniSat).
/// The Verum project is Apache-2.0 licensed, so GPL components are off by default.
#[cfg(any(feature = "vendored", feature = "static"))]
fn feature_gpl(on: &'static str, off: &'static str) -> &'static str {
    #[cfg(feature = "gpl")]
    return on;
    #[cfg(not(feature = "gpl"))]
    return off;
}

/// Convert a boolean to CMake's "ON"/"OFF" string.
#[cfg(any(feature = "vendored", feature = "static"))]
fn bool_define(b: bool) -> &'static str {
    if b { "ON" } else { "OFF" }
}

/// Return the number of logical CPU cores on the build machine.
///
/// Falls back to 4 if detection fails (reasonable default for CI).
fn num_logical_cpus() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4)
}
