//! End-to-end integration tests for `verum cache-closure`.
//!
//! Spawns the actual `verum` binary and validates the chain:
//!
//!   `verum cache-closure {stat,list,get,clear,decide}` (clap) →
//!     `commands::cache_closure::run_*` →
//!       `verum_verification::closure_cache::FilesystemCacheStore` →
//!         per-theorem JSON record on disk
//!
//! Together with the 26 trait-level tests in
//! `verum_verification::closure_cache::tests` and the 14 handler
//! unit tests in `commands::cache_closure::tests`, this proves the
//! `IncrementalCacheStore` trait surface is consumable from a shell
//! — closing the integration gap #79 was opened to address.
//!
//! ## Test fixture pattern
//!
//! Every test points the cache at a tempdir via `--root <dir>`, so
//! the test never touches the developer's actual `target/.verum_cache/`.

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn verum_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_verum"))
}

/// Open a fresh tempdir for use as the cache root.  The TempDir
/// must be kept alive for the test's duration.
fn fresh_root() -> (TempDir, String) {
    let t = TempDir::new().expect("tempdir");
    let p = t.path().to_string_lossy().into_owned();
    (t, p)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(verum_bin())
        .args(args)
        .output()
        .expect("CLI invocation must succeed")
}

// ─────────────────────────────────────────────────────────────────────
// stat / list on an empty cache
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_stat_empty_cache_reports_zero_entries() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "stat",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["entries"], 0);
    assert_eq!(parsed["hits"], 0);
    assert_eq!(parsed["misses"], 0);
}

#[test]
fn cache_list_empty_cache_succeeds() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "list",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["count"], 0);
}

// ─────────────────────────────────────────────────────────────────────
// decide flow — the user-facing skip / recheck contract
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_decide_no_entry_returns_recheck_no_cache_entry() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "decide",
        "thm.absent",
        "--signature",
        "sig",
        "--body",
        "body",
        "--kernel-version",
        "2.6.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["decision"]["action"], "recheck");
    assert_eq!(parsed["decision"]["reason"], "no_cache_entry");
}

#[test]
fn cache_decide_rejects_empty_signature() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "",
        "--body",
        "body",
        "--root",
        &root,
    ]);
    assert!(!out.status.success(), "empty --signature must error");
}

#[test]
fn cache_decide_rejects_empty_body() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "",
        "--root",
        &root,
    ]);
    assert!(!out.status.success(), "empty --body must error");
}

#[test]
fn cache_decide_outputs_64_char_closure_hash() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--cite",
        "framework_a",
        "--cite",
        "framework_b",
        "--kernel-version",
        "2.6.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let h = parsed["closure_hash"].as_str().unwrap();
    assert_eq!(h.len(), 64, "closure hash must be 64-char hex");
    assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn cache_decide_citations_order_independent() {
    // Same citation set in different order ⇒ same closure hash.
    let (_t, root) = fresh_root();
    let p = |args: &[&str]| {
        let out = run(args);
        assert!(out.status.success());
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        parsed["closure_hash"].as_str().unwrap().to_string()
    };
    let h1 = p(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--cite",
        "alpha",
        "--cite",
        "beta",
        "--kernel-version",
        "2.6.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    let h2 = p(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--cite",
        "beta",
        "--cite",
        "alpha",
        "--kernel-version",
        "2.6.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert_eq!(h1, h2, "citation order must not affect hash");
}

#[test]
fn cache_decide_kernel_version_drift_changes_hash() {
    let (_t, root) = fresh_root();
    let p = |args: &[&str]| {
        let out = run(args);
        assert!(out.status.success());
        let parsed: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
        parsed["closure_hash"].as_str().unwrap().to_string()
    };
    let h_a = p(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--kernel-version",
        "2.6.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    let h_b = p(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--kernel-version",
        "2.7.0",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert_ne!(
        h_a, h_b,
        "kernel-version drift must change the closure hash"
    );
}

// ─────────────────────────────────────────────────────────────────────
// get on missing theorem → error
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_get_missing_theorem_errors() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "get",
        "thm.absent",
        "--root",
        &root,
    ]);
    assert!(
        !out.status.success(),
        "get on missing theorem must produce non-zero exit"
    );
}

// ─────────────────────────────────────────────────────────────────────
// clear is idempotent
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_clear_empty_succeeds() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "clear",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    assert!(out.status.success());
    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(parsed["cleared"], 0);
}

// ─────────────────────────────────────────────────────────────────────
// Format validation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_stat_rejects_unknown_format() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "stat",
        "--root",
        &root,
        "--format",
        "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn cache_list_rejects_unknown_format() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "list",
        "--root",
        &root,
        "--format",
        "yaml",
    ]);
    assert!(!out.status.success());
}

#[test]
fn cache_decide_rejects_unknown_format() {
    let (_t, root) = fresh_root();
    let out = run(&[
        "cache-closure",
        "decide",
        "thm.x",
        "--signature",
        "sig",
        "--body",
        "body",
        "--root",
        &root,
        "--format",
        "yaml",
    ]);
    assert!(!out.status.success());
}

// ─────────────────────────────────────────────────────────────────────
// Cross-endpoint consistency: stat counts agree with list count
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cache_stat_entries_matches_list_count_after_decides() {
    // Run several decide commands → each produces a Recheck verdict
    // with no_cache_entry (cache stays empty).  stat.entries must
    // remain at 0; list.count likewise.  This pins the contract
    // that `decide` is a *probe* — it never writes.
    let (_t, root) = fresh_root();
    for thm in ["thm.a", "thm.b", "thm.c"] {
        let out = run(&[
            "cache-closure",
            "decide",
            thm,
            "--signature",
            "sig",
            "--body",
            "body",
            "--root",
            &root,
            "--format",
            "json",
        ]);
        assert!(out.status.success());
    }
    let stat_out = run(&[
        "cache-closure",
        "stat",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    let stat: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&stat_out.stdout)).unwrap();
    let list_out = run(&[
        "cache-closure",
        "list",
        "--root",
        &root,
        "--format",
        "json",
    ]);
    let list: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&list_out.stdout)).unwrap();
    assert_eq!(stat["entries"], list["count"]);
    assert_eq!(stat["entries"], 0);
}
