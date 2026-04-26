//! Ratchet: count lenient `SKIP` warnings emitted during a stdlib-loading
//! compile and fail CI if the count grows.
//!
//! Each `[lenient] SKIP <fn>: <reason>` is a body that VBC codegen
//! could not compile and dropped silently — runtime calls to that
//! function panic with `method 'X.Y' not found on value` or
//! `FunctionNotFound(...)` far from the cause.  The lenient surface is
//! the most direct measurement of stdlib hygiene.
//!
//! Earlier fixes drove the count from ~50 to 0 on a tiny bare-`None`
//! fixture (#158, #161, #159).  This test pins that baseline at zero
//! and forces any new stdlib bug that introduces a SKIP to land its
//! own task / fix before the PR can merge.
//!
//! When this fails, look at the warning text:
//!   * `undefined function: <name>` → real missing function or
//!     mount-alias not propagating; add the function or de-alias the
//!     mount (#159 pattern).
//!   * `undefined variable: <Variant>` → cross-type variant collision
//!     dropping the simple-name alias.  Either ensure the colliding
//!     types have unique simple names (#160 / `stdlib_unique_type_names`)
//!     or check `register_type_constructors` (#158 `prefer_existing`
//!     save/restore guard).
//!   * `wrong number of arguments for <name>` → arity-suffix
//!     registration regression; check
//!     `crates/verum_vbc/src/codegen/context.rs::register_function`
//!     keeps both arities (#161).

use std::path::PathBuf;
use std::process::Command;

const FIXTURE: &str = r#"// @test: run-interpreter
// @tier: 0
// @level: L0
// @timeout: 20000
// @expected-stdout: ok
// @expected-exit: 0

fn main() {
    let m: Maybe<Int> = None;
    match m {
        None    => print("ok"),
        Some(_) => panic("Some on None"),
    }
}
"#;

fn workspace_root() -> PathBuf {
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

fn locate_vtest(root: &std::path::Path) -> PathBuf {
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

/// Run vtest on `target_path` with `RUST_LOG=warn` and return any
/// `[lenient] SKIP` warning lines.
///
/// vtest's tracing subscriber lands the lenient-skip warn-lines on
/// **stdout** when stderr is a pipe (Command::output captures), even
/// though `tracing_subscriber::fmt::layer()` defaults to stderr — the
/// vtest test-executor relays subprocess output through stdout for
/// per-test reporting.  We scan both streams to be robust against
/// either routing path.
fn collect_lenient_skips(target_path: &std::path::Path) -> (Option<i32>, Vec<String>) {
    let root = workspace_root();
    let vtest = locate_vtest(&root);
    let output = Command::new(&vtest)
        .args(["run", target_path.to_str().unwrap()])
        .env("RUST_LOG", "warn")
        .current_dir(&root)
        .output()
        .expect("failed to run vtest");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let skips: Vec<String> = stderr
        .lines()
        .chain(stdout.lines())
        .filter(|l| l.contains("[lenient]") && l.contains("SKIP"))
        .map(|l| l.to_string())
        .collect();
    (output.status.code(), skips)
}

const FAILURE_HINT: &str =
    "Each SKIP is a stdlib body that VBC codegen could not compile and \
     dropped silently, surfacing later as `method 'X.Y' not found on \
     value` or `FunctionNotFound(...)` runtime panics.\n\n\
     Most likely diagnostic classes:\n\
     * `undefined function: NAME` — the function isn't registered, \
       usually a missing mount or a mount-alias that didn't propagate \
       into the codegen function table.\n\
     * `undefined variable: VARIANT` — cross-type variant collision \
       wiping the simple-name alias.  See #158/#160 fixes in \
       `register_type_constructors`.\n\
     * `wrong number of arguments for NAME` — arity-suffix registration \
       was bypassed under `prefer_existing_functions`.  See \
       `register_function` in `codegen/context.rs` (#161).";

/// Run vtest on `spec` and assert it emits zero `[lenient] SKIP`
/// warnings during stdlib loading.  `scenario` names the fixture in
/// the failure message so a CI diff identifies which spec regressed.
///
/// If the spec file is missing the assertion is skipped silently —
/// vcs/ specs are optional in some workspace layouts and this test
/// must not block builds where the fixture isn't present.  The
/// `expect_present` flag (set by the SQLite-VFS fixture which is
/// load-bearing) overrides that and panics on a missing fixture.
fn assert_no_lenient_skips(scenario: &str, spec: &std::path::Path, expect_present: bool) {
    if !spec.is_file() {
        if expect_present {
            panic!("expected {} smoke at {} but it is missing", scenario, spec.display());
        }
        return;
    }
    let (code, skips) = collect_lenient_skips(spec);
    assert!(
        skips.is_empty(),
        "{} smoke triggered {} lenient `SKIP` warning(s) during stdlib \
         loading (exit code: {:?}).\n\n{}\n\nFirst few warnings:\n{}",
        scenario,
        skips.len(),
        code,
        FAILURE_HINT,
        skips.iter().take(8).map(|s| s.as_str()).collect::<Vec<_>>().join("\n"),
    );
}

#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_minimal() {
    // Minimal fixture is created on the fly so the test is
    // self-contained — no spec file dependency outside this crate.
    let root = workspace_root();
    let dir = root.join("vcs/specs/L0-critical/_codegen_regressions");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join("lenient_skip_baseline.vr");
    std::fs::write(&fixture, FIXTURE).expect("write fixture");

    let (_code, skips) = collect_lenient_skips(&fixture);

    let _ = std::fs::remove_file(&fixture);
    let _ = std::fs::remove_dir(&dir);

    assert!(
        skips.is_empty(),
        "stdlib loading emitted {} lenient `SKIP` warning(s) on the minimal \
         bare-`None` fixture.\n\n{}\n\nFirst few warnings:\n{}",
        skips.len(),
        FAILURE_HINT,
        skips.iter().take(8).map(|s| s.as_str()).collect::<Vec<_>>().join("\n"),
    );
}

/// Wider coverage check: run a real, dependency-heavy VCS spec through
/// vtest and assert the stdlib body-compilation pass remains lenient-
/// skip-free.  The minimal fixture above only loads the core slice;
/// this one transitively pulls in the SQLite VFS layer + collections +
/// I/O — the historical hot-path for codegen-hygiene regressions.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_sqlite() {
    let target = workspace_root().join(
        "vcs/specs/L2-standard/database/sqlite/l0_vfs/memdb_open_write_read.vr",
    );
    assert_no_lenient_skips("SQLite-VFS", &target, /* expect_present */ true);
}

/// Even-wider coverage: the L1 pager round-trip pulls in sys.time_ops
/// (Instant.now / sleep_*), the rollback journal helpers, the WAL
/// frame layout, plus the L0 VFS layer that the SQLite-VFS smoke
/// already exercises.  Covers the historical hot-paths for both the
/// "missing FFI intrinsic" cluster (`__time_*_nanos_raw`) and the
/// "rollback record helpers not exported" cluster.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_l1_pager() {
    let target = workspace_root().join(
        "vcs/specs/L2-standard/database/sqlite/l1_pager/page_roundtrip.vr",
    );
    assert_no_lenient_skips("L1-pager", &target, false);
}

/// Top-of-stack coverage: an L4 VDBE program-builder smoke pulls in
/// the entire SQLite native stack from L0 (VFS) through L3 (btree)
/// and L4 (VDBE interpreter + opcode catalogue + register file).
/// Specifically exercises the post-#162 renamed types in the L4
/// surface — `Opcode` (l4_vdbe canonical), `Register` (l4_vdbe
/// canonical), `StepResult` (l4_vdbe canonical), `VdbeFrame`
/// (vdbe_subprogram_api), `Affinity` (l2_record canonical), plus the
/// transitively-pulled status / opcode_catalog / l3_btree types —
/// catching regressions in any L0..L4 module that reaches stdlib
/// codegen via the `simple_select_program` test path.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_l4_vdbe() {
    let target = workspace_root().join(
        "vcs/specs/L2-standard/database/sqlite/l4_vdbe/simple_select_program.vr",
    );
    assert_no_lenient_skips("L4-VDBE", &target, false);
}

/// Cross-domain coverage: runtime/recovery + retry primitives.  This
/// fixture pulls in a different stdlib slice from the SQLite-heavy
/// fixtures above — async/spawn_config, runtime/recovery (the
/// supervisor-tree-side counterpart to async/spawn_config), the
/// `RecoveryBackoffStrategy` / `RuntimeRetryConfig` / `JitterConfig`
/// / `RetryPredicate` types that landed renamed during #162 — plus
/// the foundational base/result and core.* infrastructure that
/// async-related modules transitively need.
///
/// Specifically guards against any regression that re-introduces a
/// stdlib-loading SKIP in `core/runtime/recovery.vr`,
/// `core/async/spawn_config.vr`, or any of the runtime/* siblings
/// that share their type registrations with these modules.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_runtime_retry() {
    let target = workspace_root().join("vcs/specs/core/runtime/retry_minimal_test.vr");
    assert_no_lenient_skips("runtime/retry", &target, false);
}

/// Text-formatting subgraph coverage: pulls in core/text/format
/// (the printf-style formatter renamed to `TextFormatter` during
/// #162), the format protocol surface (LowerHex / UpperHex / Binary
/// / Octal / LowerExp / UpperExp), the `FormatSpec` / `FormatError`
/// types, plus the foundational base/protocols::Formatter
/// (Display/Debug) which is the canonical sibling kept under the
/// disambiguation.  Catches regressions in any of the format-related
/// module entries that share their type registrations with
/// core/text/format.vr or core/base/protocols.vr.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_text_format() {
    let target = workspace_root().join("vcs/specs/core/text/format_traits_test.vr");
    assert_no_lenient_skips("text/format", &target, false);
}

/// I/O protocols subgraph coverage: pulls in core/io/protocols
/// (the foundational `Read` / `Write` / `Seek` traits + StreamError
/// + IoErrorKind taxonomy + the renamed `EmptyReader` from #162's
/// Empty<T> disambiguation), plus core/io/path (canonical `Path`
/// after net/quic/path → QuicPath rename and math/hott → HottPath
/// rename), and the ByteRepeat / Sink utility readers/writers.
///
/// Sits alongside the runtime/retry and text/format fixtures as
/// independent non-SQLite stdlib subgraph coverage — a regression
/// in any io.* module that affects type-table population fails
/// here without depending on SQLite codegen at all.
#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_emits_no_lenient_skips_io_protocols() {
    let target = workspace_root().join("vcs/specs/core/io/protocols_test.vr");
    assert_no_lenient_skips("io/protocols", &target, false);
}
