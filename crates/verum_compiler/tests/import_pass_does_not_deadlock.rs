//! The type-check phase must not hold a registry read guard across an
//! import that can lazy-load.
//!
//! `verum check core/intrinsics/mod.vr` never returned. Sampled after 17
//! minutes on that one file:
//!
//! ```text
//! phase_type_check          phases_orchestration.rs:610
//!   process_import          modules.rs:2116
//!     ..._body              modules.rs:3258   session_registry.write()
//!       RawRwLock::lock_exclusive_slow -> wait_for_readers
//! ```
//!
//! Only one thread was doing work, so the readers it waited for were its
//! own. `checker.process_import(…, &registry.read())` keeps the guard
//! alive for the whole call — a temporary in argument position lives to
//! the end of the statement — and deep inside, the lazy loader registers
//! what it resolved under a WRITE lock on the same registry.
//! `parking_lot`'s RwLock is not reentrant.
//!
//! It became reachable when `set_module_registry` started sharing ONE
//! handle for `module_registry` and `session_registry`, deliberately, to
//! stop the two copies drifting as lazy loads landed in one and not the
//! other. A repair for a DRIFT bug turned a harmless lock on two objects
//! into a self-deadlock on one.
//!
//! WHY THIS TEST NAMES A REAL FILE INSTEAD OF SCAFFOLDING ONE. Three
//! synthetic projects were built and measured against the pre-fix
//! binary, and all three finished in about 0.25s:
//!
//!   * an aggregator at the project root with two relative glob mounts;
//!   * the same one directory down, matching core/'s own layout;
//!   * a user file mounting `core.intrinsics.arithmetic.*` directly.
//!
//! So did `core/collections/mod.vr`. The lazy resolver engages only for
//! a stdlib path that is not already registered, and reproducing that
//! took the exact shape of `core/intrinsics/mod.vr`. A scaffold that
//! does not reproduce is not a lighter test — it is a test that passes
//! for the wrong reason, and this one is checked: the pre-fix binary
//! exits 124 on the file below and 0 on every scaffold tried.
//!
//! If the file is ever moved or thinned out, this test should fail
//! loudly rather than be repointed at whatever is nearest: re-derive the
//! trigger by sweeping core/ and sampling whatever stops advancing.
//!
//! WHY A TIMEOUT AND NOT AN ASSERTION ON OUTPUT: a deadlock has no
//! output. The only observable is that the process does not finish, so
//! the test has to be able to give up — and a test that hangs the suite
//! to report a hang is not a test.
//!
//! Task: T0926.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Generous next to the ~2s this check takes when it works, and far
/// under the 17 minutes the deadlock ran before it was sampled.
const BUDGET: Duration = Duration::from_secs(120);

/// `core/intrinsics/mod.vr`, the one file measured to reproduce.
const AGGREGATOR: &str = "core/intrinsics/mod.vr";

fn repo_root() -> PathBuf {
    // crates/verum_compiler -> crates -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root above crates/verum_compiler")
        .to_path_buf()
}

fn verum_binary() -> PathBuf {
    let exe = std::env::current_exe().expect("current exe");
    let mut dir = exe.parent();
    while let Some(d) = dir {
        let candidate = d.join("verum");
        if candidate.is_file() {
            return candidate;
        }
        dir = d.parent();
    }
    panic!(
        "no `verum` binary in any ancestor of this test binary — build one \
         with `cargo build -p verum_cli --release` before running this gate"
    )
}

#[test]
fn checking_the_intrinsics_aggregator_finishes() {
    let root = repo_root();
    let target = root.join(AGGREGATOR);
    assert!(
        target.is_file(),
        "{AGGREGATOR} is gone. This test names a real file because no \
         synthetic project reproduced the deadlock (see the module docs). \
         Do not repoint it at a nearby file without checking that the new \
         one reproduces against a pre-fix binary — re-derive the trigger \
         by sweeping core/ and sampling whatever stops advancing."
    );

    let verum = verum_binary();
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let out = Command::new(&verum)
            .arg("check")
            .arg(AGGREGATOR)
            .current_dir(&root)
            .stdin(Stdio::null())
            .output();
        let _ = tx.send(out.map(|o| o.status.code()));
    });

    match rx.recv_timeout(BUDGET) {
        Ok(Ok(_code)) => {
            // Whether the check PASSES is another file's business. What
            // this one asserts is that it comes back at all.
            let _ = handle.join();
        }
        Ok(Err(e)) => panic!("could not run `verum check`: {e}"),
        Err(_) => panic!(
            "`verum check {AGGREGATOR}` did not finish in {BUDGET:?} — the \
             import pass is holding a registry read guard across a lazy load \
             again (T0926). Sample the process: the stack ends in \
             RawRwLock::wait_for_readers under process_import."
        ),
    }
}
