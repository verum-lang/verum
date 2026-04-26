//! Regression contract for cross-module same-name / different-arity
//! function registration during stdlib loading.
//!
//! Background.  Several stdlib symbols share simple names but live in
//! different modules with different arities — the canonical pair is
//! `core/sys/<plat>/libsystem.vr::write(fd, buf, n)` (3-arity FFI binding)
//! and `core/io/fs.vr::write(path, contents)` (2-arity high-level helper).
//! The codegen handles this with arity-suffixed alternative keys
//! (`name#arity`) and an arity-aware lookup at the call site.
//!
//! In `prefer_existing_functions = true` mode (set by pipeline.rs while
//! loading imported stdlib modules), `register_function` had a fast path
//! that skipped the arity-suffix branch entirely:
//!
//! ```ignore
//! if self.prefer_existing_functions {
//!     self.functions.entry(name).or_insert(info);    // <- silent drop
//! } else { ... store name or alt-key based on arity ... }
//! ```
//!
//! The second registration was simply discarded.  When `core/io/fs.vr`
//! loaded before `libsystem.vr`, the FFI 3-arity `write` was dropped and
//! every `safe_write`, `safe_pread`, `safe_pwrite`, `safe_send`,
//! `safe_recv`, `safe_sendto`, `safe_getsockopt` wrapper failed to
//! compile with `wrong number of arguments for write: expected 2, found 3`
//! and disappeared via the lenient-skip path — surfacing later as
//! `method 'X.Y' not found on value` runtime panics far from the cause.
//!
//! The fix moves the arity-suffix branch outside the prefer-existing
//! gate so both modes preserve alternative arities (first-wins under
//! prefer-existing, last-wins otherwise).
//!
//! This test pins the fix in place: it runs vtest on a fixture that
//! transitively loads the stdlib (any `// @test: run-interpreter` does)
//! and asserts no `wrong number of arguments for write|pread|pwrite|
//! send|recv|sendto|getsockopt` lenient warnings appear in stderr.

mod stdlib_support;
use stdlib_support::{vtest_run_capture, workspace_root};

const FIXTURE: &str = r#"// @test: run-interpreter
// @tier: 0
// @level: L0
// @timeout: 20000
// @expected-stdout: ok
// @expected-exit: 0

fn main() {
    print("ok");
}
"#;

/// FFI builtins that are known to share simple names with high-level
/// stdlib helpers of different arity. Each entry must keep both arities
/// resolvable for the safe wrappers in `core/sys/<plat>/libsystem.vr`
/// to compile.
const ARITY_SENSITIVE_NAMES: &[&str] = &[
    "write", "pread", "pwrite",
    "send", "recv", "sendto",
    "getsockopt", "safe_getsockopt",
];

#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_preserves_alternative_arities() {
    let root = workspace_root();
    let dir = root.join("vcs/specs/L0-critical/_codegen_regressions");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join("arity_disambiguation_preserved.vr");
    std::fs::write(&fixture, FIXTURE).expect("write fixture");

    let out = vtest_run_capture(&fixture);

    let arity_warnings: Vec<&str> = out.stderr
        .lines()
        .filter(|l| l.contains("[lenient]") && l.contains("wrong number of arguments"))
        .filter(|l| ARITY_SENSITIVE_NAMES.iter().any(|n| {
            l.contains(&format!("for {}:", n))
        }))
        .collect();

    assert!(
        arity_warnings.is_empty(),
        "stdlib loading dropped an alternative-arity registration — {} \
         FFI/wrapper body compilation(s) failed with `wrong number of \
         arguments`.\n\n\
         Root cause class: `register_function` ran in \
         `prefer_existing_functions = true` mode and short-circuited \
         through `entry(name).or_insert(info)`, silently discarding the \
         second registration when the existing entry had a different \
         arity. The arity-suffix branch (`name#arity`) is what makes \
         multi-arity simple names work — it must run in BOTH modes.\n\n\
         Look in `crates/verum_vbc/src/codegen/context.rs::register_function` \
         for the arity-suffix branch outside the prefer-existing gate.\n\n\
         First few warnings:\n{}",
        arity_warnings.len(),
        arity_warnings.iter().take(8).copied().collect::<Vec<_>>().join("\n"),
    );

    assert_eq!(
        out.exit_code,
        Some(0),
        "fixture exited with non-zero; stderr:\n{}",
        out.stderr,
    );

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_dir(&dir);
}
