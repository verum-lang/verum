//! Contract tests for `resolved_stdlib_identity` (T0523) — the script
//! VBC cache-key contributor that makes a core/ edit invalidate cached
//! bytecode instead of silently rerunning stale green.
//!
//! NOTE: CI currently runs `--lib --bins` and skips integration tests
//! (inert until T0709); the contract still runs locally.
//!
//! The env-var branch is exercised through a private temp dir; tests in
//! this file are serialized by taking the same mutex so the VERUM_STDLIB_PATH
//! mutation cannot race a parallel test.

use std::sync::Mutex;

use verum_compiler::embedded_stdlib_vbc::resolved_stdlib_identity;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard(Option<String>);
impl EnvGuard {
    fn set(dir: &std::path::Path) -> Self {
        let prev = std::env::var("VERUM_STDLIB_PATH").ok();
        // SAFETY: single-threaded within ENV_LOCK.
        unsafe { std::env::set_var("VERUM_STDLIB_PATH", dir) };
        EnvGuard(prev)
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(v) => unsafe { std::env::set_var("VERUM_STDLIB_PATH", v) },
            None => unsafe { std::env::remove_var("VERUM_STDLIB_PATH") },
        }
    }
}

/// Editing ANY stdlib source under a VERUM_STDLIB_PATH override changes
/// the identity — the acceptance's "rerunning the same unchanged script
/// either recompiles or refuses the cache hit" reduces to exactly this
/// (the identity is a key contributor).
#[test]
fn source_override_identity_tracks_content() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("t0523-id-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("base")).unwrap();
    std::fs::write(dir.join("base/a.vr"), "fn one() -> Int { 1 }\n").unwrap();
    let _env = EnvGuard::set(&dir);

    let first = resolved_stdlib_identity();
    assert!(first.starts_with("src:"), "override must hash the tree, got {first}");
    let again = resolved_stdlib_identity();
    assert_eq!(first, again, "identity must be deterministic");

    std::fs::write(dir.join("base/a.vr"), "fn one() -> Int { 2 }\n").unwrap();
    let edited = resolved_stdlib_identity();
    assert_ne!(first, edited, "editing a stdlib source must change the identity");

    std::fs::write(dir.join("base/b.vr"), "fn two() -> Int { 2 }\n").unwrap();
    let added = resolved_stdlib_identity();
    assert_ne!(edited, added, "adding a stdlib source must change the identity");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Without the override the identity comes from the embedded archive:
/// stable within a binary, and never the empty string (a cache key must
/// always carry SOME stdlib fact — "none" is the honest no-archive form).
#[test]
fn embedded_identity_is_stable_and_tagged() {
    let _g = ENV_LOCK.lock().unwrap();
    let prev = std::env::var("VERUM_STDLIB_PATH").ok();
    unsafe { std::env::remove_var("VERUM_STDLIB_PATH") };
    let a = resolved_stdlib_identity();
    let b = resolved_stdlib_identity();
    assert_eq!(a, b);
    assert!(
        a.starts_with("emb:") || a == "none",
        "expected emb:<digest> or none, got {a}"
    );
    if let Some(v) = prev {
        unsafe { std::env::set_var("VERUM_STDLIB_PATH", v) };
    }
}
