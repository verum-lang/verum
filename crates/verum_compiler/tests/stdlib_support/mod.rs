//! Shared helpers for the `stdlib_*` integration tests.
//!
//! Each of `stdlib_lenient_skip_baseline`, `stdlib_simple_variant_alias_preservation`,
//! and `stdlib_arity_disambiguation` runs the workspace's `vtest` binary
//! against a `.vr` fixture and inspects the captured stderr/stdout for a
//! specific class of diagnostic.  The boilerplate they share — locating
//! the workspace root, locating the built `vtest` binary, capturing a
//! subprocess output — is identical across the three tests.  Centralising
//! it here removes ~60 lines of copy-paste and ensures every test agrees
//! on the binary-discovery contract.
//!
//! Why a dedicated `tests/stdlib_support/mod.rs` rather than `tests/common/`:
//! the helpers are tightly coupled to the stdlib-loading vtest invocation
//! pattern (always `vtest run <spec>`, always `RUST_LOG=warn`, always
//! capturing both streams).  Other integration tests in this crate have
//! different needs (compilation-only, in-process compiler invocation,
//! etc.) so a generic `common/` module would have to grow conflicting
//! abstractions.  Scoping the module name to `stdlib_*` makes its surface
//! and intended use site explicit.
//!
//! Visibility: every helper here is `pub(crate)`-equivalent (`pub`
//! within this integration-test crate).  Items are exposed even when
//! a particular test file doesn't use all of them — `#[allow(dead_code)]`
//! at the module level prevents per-test `unused` warnings since
//! cargo compiles each test file as a separate crate that pulls this
//! module in via `mod stdlib_support;`.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;

/// Locate the workspace root (directory containing `Cargo.lock` and
/// the `core/` stdlib).  Walks up from this crate's manifest dir.
pub fn workspace_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in crate_dir.ancestors() {
        if ancestor.join("Cargo.lock").is_file() && ancestor.join("core").is_dir() {
            return ancestor.to_path_buf();
        }
    }
    panic!(
        "workspace root with Cargo.lock and core/ not found from {}",
        crate_dir.display()
    );
}

/// Locate a built `vtest` binary under the workspace root.  Prefers
/// the release build (used in CI) and falls back to debug.  Panics
/// with a clear message when neither exists — the caller is expected
/// to be a `#[ignore]`-gated test that the user is opting into via
/// `cargo test -- --ignored`, so requiring the build is acceptable.
pub fn locate_vtest(root: &std::path::Path) -> PathBuf {
    let release = root.join("target/release/vtest");
    if release.is_file() {
        return release;
    }
    let debug = root.join("target/debug/vtest");
    if debug.is_file() {
        return debug;
    }
    panic!(
        "vtest binary not found at target/release/vtest or target/debug/vtest \
         under {}; run `cargo build -p vtest` first",
        root.display()
    );
}

/// Run `vtest run <spec>` with `RUST_LOG=warn` and return the exit
/// code (or `None` if the process was signalled) plus the merged
/// stderr/stdout content as one string per stream.
///
/// vtest's tracing subscriber lands warn-level lines on stdout when
/// stderr is a pipe (because the test executor relays subprocess
/// output through stdout for per-test reporting), even though
/// `tracing_subscriber::fmt::layer()` defaults to stderr.  Callers
/// scanning for log lines should therefore consider both streams.
pub fn vtest_run_capture(target_path: &std::path::Path) -> VtestOutput {
    let root = workspace_root();
    let vtest = locate_vtest(&root);
    let output = Command::new(&vtest)
        .args(["run", target_path.to_str().unwrap()])
        .env("RUST_LOG", "warn")
        .current_dir(&root)
        .output()
        .expect("failed to run vtest");
    VtestOutput {
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    }
}

/// Captured output from a vtest subprocess.  See `vtest_run_capture`
/// for why both `stderr` and `stdout` are exposed.
pub struct VtestOutput {
    pub exit_code: Option<i32>,
    pub stderr: String,
    pub stdout: String,
}

impl VtestOutput {
    /// Iterate over lines from stderr followed by stdout — the
    /// canonical scan order for log-line predicates.
    pub fn merged_lines(&self) -> impl Iterator<Item = &str> {
        self.stderr.lines().chain(self.stdout.lines())
    }

    /// Collect every merged log line that matches `pred` into a
    /// `Vec<String>`.  Callers use this to build assertion messages
    /// listing offending lines verbatim.
    pub fn lines_matching(&self, pred: impl Fn(&str) -> bool) -> Vec<String> {
        self.merged_lines()
            .filter(|l| pred(l))
            .map(str::to_string)
            .collect()
    }
}
