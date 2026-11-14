#![allow(clippy::disallowed_types)] // Build scripts can use String/Vec directly

//! Build script for verum_cli.
//!
//! Sets up environment variables for the CLI.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    emit_build_identity();
    // Export TARGET for runtime detection
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=VERUM_HOST_TARGET={}", target);
    println!("cargo:rustc-env=TARGET={}", target);

    // Provide LLVM library search path for native compilation (AOT + GPU paths).
    // This lets the linker find libLLVM, libMLIR, and libclang_rt when
    // the verum CLI compiles user programs to native executables.
    if let Some(llvm_dir) = find_llvm_install_dir() {
        let lib_dir = llvm_dir.join("lib");
        println!("cargo:rustc-env=VERUM_LLVM_LIB_DIR={}", lib_dir.display());
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VERUM_LLVM_DIR");
}

/// Find the LLVM install directory (same logic as verum_llvm_sys).
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

/// Volatile identity is owned by the final application. Shared error/compiler
/// crates have no Git or timestamp build inputs.
fn emit_build_identity() {
    let manifest =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    for (output, input) in [
        ("VERUM_BUILD_PROFILE", "PROFILE"),
        ("VERUM_BUILD_TARGET", "TARGET"),
        ("VERUM_BUILD_HOST", "HOST"),
    ] {
        emit(
            output,
            &env::var(input).expect("Cargo build identity input"),
        );
    }
    let rustc = env::var_os("RUSTC").expect("Cargo-selected rustc");
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("rustc version");
    assert!(
        version.status.success(),
        "Cargo-selected rustc version failed"
    );
    emit(
        "VERUM_BUILD_RUSTC",
        String::from_utf8_lossy(&version.stdout).trim(),
    );

    let timestamp = match env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value
            .parse::<u64>()
            .expect("SOURCE_DATE_EPOCH must be unsigned seconds"),
        Err(env::VarError::NotPresent) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("build timestamp before epoch")
            .as_secs(),
        Err(error) => panic!("invalid SOURCE_DATE_EPOCH: {error}"),
    };
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    emit("VERUM_BUILD_TIMESTAMP", &timestamp.to_string());
    emit(
        "VERUM_BUILD_GIT_SHA",
        &git_line(&manifest, &["rev-parse", "--short=12", "HEAD"])
            .unwrap_or_else(|| "unknown".into()),
    );
    let dirty = match git(&manifest, &["status", "--porcelain"]) {
        Some(status) if status.is_empty() => "clean",
        Some(_) => "dirty",
        None => "unknown",
    };
    emit("VERUM_BUILD_GIT_DIRTY", dirty);
    watch_git_inputs(&manifest);
    // Archives have no Git files to watch. Source edits must still refresh the
    // executable's timestamp; nonexistent Git paths must not force every build.
    watch_existing(&manifest.join("src"));
}

fn emit(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={}", value.replace(['\n', '\r'], " "));
}

fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(directory)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).expect("Git build-identity paths must be UTF-8"))
}

// Only line-oriented Git queries end in LF. Never trim path whitespace or
// apply text trimming to the NUL-delimited file roster.
fn git_line(directory: &Path, args: &[&str]) -> Option<String> {
    git(directory, args).map(|line| line.strip_suffix('\n').unwrap_or(&line).to_owned())
}

fn git_path(manifest: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git_line(manifest, &["rev-parse", "--git-path", name])?);
    Some(if path.is_absolute() {
        path
    } else {
        manifest.join(path)
    })
}

fn watch_existing(path: &Path) {
    if path.exists() {
        let path = path
            .to_str()
            .expect("Cargo build-identity paths must be UTF-8");
        assert!(
            !path.contains(['\n', '\r']) && path.trim_end() == path,
            "Cargo cannot watch build-identity path without changing its spelling: {path:?}"
        );
        println!("cargo:rerun-if-changed={path}");
    }
}

fn watch_git_inputs(manifest: &Path) {
    let Some(root) = git_line(manifest, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
    else {
        return;
    };
    for name in ["HEAD", "index"] {
        if let Some(path) = git_path(manifest, name) {
            watch_existing(&path);
        }
    }
    if let Some(branch) = git_line(manifest, &["symbolic-ref", "--quiet", "HEAD"])
        && let Some(path) = git_path(manifest, &branch)
    {
        if path.is_file() {
            watch_existing(&path);
        } else {
            if let Some(packed) = git_path(manifest, "packed-refs") {
                watch_existing(&packed);
            }
            // A packed branch can acquire a loose ref on the next commit.
            if let Some(parent) = path.ancestors().skip(1).find(|p| p.is_dir()) {
                watch_existing(parent);
            }
        }
    }
    // Track actual source inputs so a dependency edit updates dirty status even
    // when only that dependency, rather than the CLI's own source, changed.
    if let Some(files) = git(
        &root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        for name in files.split('\0').filter(|name| !name.is_empty()) {
            let path = root.join(name);
            if path.exists() {
                watch_existing(&path);
            } else if let Some(parent) = path.ancestors().skip(1).find(|p| p.is_dir()) {
                watch_existing(parent);
            }
        }
    }
}
