//! Regression contract for the simple-variant-alias preservation invariant.
//!
//! Background. Verum stdlib registers a small set of "convenience aliases"
//! (`None`, `Some`, `Ok`, `Err`, `Less`, `Equal`, `Greater`, …) so that bare
//! identifiers resolve to the right `MakeVariant` instruction at codegen
//! time.  These aliases are critical: every stdlib body that says
//! `return None` or `Poll.Ready(None)` (BTreeMap, Receiver.poll, every
//! Stream adapter, etc.) lowers through that simple-name lookup.
//!
//! Several stdlib types coincidentally declare a `None` variant of their own
//! — `RecoveryStrategy`, `BackoffStrategy`, `JitterConfig`, `LockKind` (see
//! `core/runtime/recovery.vr`, `core/async/spawn_config.vr`,
//! `core/database/sqlite/native/l0_vfs/vfs_protocol.vr`, etc.).  Without the
//! save/restore guard added in `crates/verum_vbc/src/codegen/mod.rs`,
//! processing any one of those types during stdlib loading flips
//! `prefer_existing_functions` back to `false` (legacy behaviour of the
//! protocol-impl branch), at which point the next `ItemKind::Type` runs the
//! cross-type collision-detection in user-mode and *unregisters* the bare
//! `None` alias.  Subsequent stdlib bodies fail to compile with
//!
//!   [lenient] SKIP <Method>: undefined variable: None
//!
//! and disappear from the runtime function table — so callers panic with
//! `method 'X.Y' not found on value` later, far from the original bug.
//!
//! This test pins the fix in place: it spawns a vtest run on a tiny fixture
//! that uses bare `None`, captures stderr, and fails CI if even one
//! `undefined variable: None` warning appears.  Any future change that
//! drops the save/restore (or otherwise re-introduces cross-type stdlib
//! collisions on built-in aliases) will fail this test instead of silently
//! dropping stdlib methods.
//!
//! Spec: `crates/verum_types/src/CLAUDE.md` — "Variant constructors:
//! User-defined variant names must freely override built-in convenience
//! aliases.  Only protect variants from ALREADY-REGISTERED types in the
//! environment, never via hardcoded name lists."

mod stdlib_support;
use stdlib_support::{vtest_run_capture, workspace_root};

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

#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_preserves_simple_none_alias() {
    let root = workspace_root();
    let dir = root.join("vcs/specs/L0-critical/_codegen_regressions");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join("simple_none_alias_preserved.vr");
    std::fs::write(&fixture, FIXTURE).expect("write fixture");

    let out = vtest_run_capture(&fixture);

    // Only inspect stderr here — the symptom is a body-compilation
    // warning emitted by the stdlib-loading codegen subscriber, which
    // routes to stderr in the fixture-style invocation (no pipe
    // remapping).  vtest_run_capture preserves both streams; using
    // `out.stderr` directly keeps this test scoped to the stderr
    // channel where the diagnostic is known to land.
    let none_warnings: Vec<&str> = out.stderr
        .lines()
        .filter(|l| l.contains("[lenient]") && l.contains("undefined variable: None"))
        .collect();

    let pass = none_warnings.is_empty() && out.exit_code == Some(0);
    if pass {
        // Cleanup only on success — a failed run leaves the fixture
        // in place for manual inspection.
        let _ = std::fs::remove_file(&fixture);
        let _ = std::fs::remove_dir(&dir);
    }

    assert!(
        none_warnings.is_empty(),
        "stdlib loading dropped the simple `None` alias — {} body \
         compilation(s) failed with `undefined variable: None`.\n\n\
         Root cause class: a stdlib type with a `None` variant ran with \
         `prefer_existing_functions == false` and tripped the cross-type \
         collision-detection in `register_type_constructors`, which \
         unregistered the bare `None` alias that other stdlib bodies \
         (BTreeMap, Receiver.poll, every Stream adapter, …) use to lower \
         `Maybe.None`.\n\n\
         Look in `crates/verum_vbc/src/codegen/mod.rs` for the \
         save/restore of `prev_prefer_existing` around the impl-block, \
         and the `prefer_existing_functions` arm of the simple-name \
         registration path.  The fix preserves caller-set context across \
         impl boundaries so the stdlib-loading `true` set by \
         pipeline.rs::compile_ast_to_vbc survives every protocol impl in \
         every imported module.\n\n\
         First few warnings:\n{}",
        none_warnings.len(),
        none_warnings.iter().take(8).copied().collect::<Vec<_>>().join("\n"),
    );

    assert_eq!(
        out.exit_code,
        Some(0),
        "fixture exited with non-zero; stderr:\n{}",
        out.stderr,
    );
}
