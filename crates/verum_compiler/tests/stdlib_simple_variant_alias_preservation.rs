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

fn run_fixture(fixture_path: &std::path::Path) -> (Option<i32>, String, String) {
    let root = workspace_root();
    let vtest = locate_vtest(&root);
    let output = Command::new(&vtest)
        .args(["run", fixture_path.to_str().unwrap()])
        .current_dir(&root)
        .output()
        .expect("failed to run vtest");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
#[ignore = "requires built target/{release,debug}/vtest; run with --ignored"]
fn stdlib_loading_preserves_simple_none_alias() {
    let root = workspace_root();
    let dir = root.join("vcs/specs/L0-critical/_codegen_regressions");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let fixture = dir.join("simple_none_alias_preserved.vr");
    std::fs::write(&fixture, FIXTURE).expect("write fixture");

    let (code, _stdout, stderr) = run_fixture(&fixture);

    // Cleanup — but only on success, so a failed run leaves the fixture
    // in place for manual inspection.
    let cleanup = |fixture: &std::path::Path, dir: &std::path::Path| {
        let _ = std::fs::remove_file(fixture);
        let _ = std::fs::remove_dir(dir);
    };

    let none_warnings: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("[lenient]") && l.contains("undefined variable: None"))
        .collect();

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
        code,
        Some(0),
        "fixture exited with non-zero; stderr:\n{}",
        stderr,
    );

    cleanup(&fixture, &dir);
}
