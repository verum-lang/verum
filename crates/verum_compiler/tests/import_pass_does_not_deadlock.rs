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
//! into a self-deadlock on one — which is why the shape is worth a test
//! rather than a comment.
//!
//! THE TRIGGER IS A RELATIVE GLOB MOUNT of a sibling that is not already
//! registered, which is exactly how every `mod.vr` in core/ is written.
//! Three call sites had the shape (`phases_orchestration.rs`,
//! `cross_file.rs`, `audit.rs`); this checks the first, which is the one
//! a plain `verum check` goes through.
//!
//! WHY A TIMEOUT AND NOT AN ASSERTION ON OUTPUT: a deadlock has no
//! output. The only observable is that the process does not finish, so
//! the test has to be able to give up — and a test that hangs the suite
//! to report a hang is not a test.
//!
//! Task: T0926.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Generous next to the ~1s this check takes when it works, and far
/// under the 17 minutes the deadlock ran before it was sampled.
const BUDGET: Duration = Duration::from_secs(90);

fn write(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create project dir");
    }
    std::fs::write(path, body).expect("write project file");
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

fn scaffold() -> PathBuf {
    let root = std::env::temp_dir().join("verum-import-deadlock");
    let _ = std::fs::remove_dir_all(&root);

    write(
        &root.join("verum.toml"),
        "[cog]\nname = \"deadlock\"\nversion = \"0.1.0\"\nedition = \"2026\"\n",
    );
    // The aggregator: relative glob mounts of siblings that are not in
    // the registry when this file is checked on its own, so each one
    // reaches the lazy loader. Two of them, because one module resolving
    // is not evidence that a second can follow it.
    write(
        &root.join("mod.vr"),
        "module deadlock;\n\
         public mount alpha.*;\n\
         public mount beta.*;\n",
    );
    write(
        &root.join("alpha.vr"),
        "module deadlock.alpha;\n\
         public fn a() -> Int { 1 }\n",
    );
    write(
        &root.join("beta.vr"),
        "module deadlock.beta;\n\
         public fn b() -> Int { 2 }\n",
    );
    root
}

#[test]
fn checking_an_aggregator_module_finishes() {
    let verum = verum_binary();
    let root = scaffold();

    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let out = Command::new(&verum)
            .arg("check")
            .arg("mod.vr")
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
            "`verum check mod.vr` did not finish in {BUDGET:?} — the import \
             pass is holding a registry read guard across a lazy load again \
             (T0926). Sample the process: the stack ends in \
             RawRwLock::wait_for_readers under process_import."
        ),
    }
}
