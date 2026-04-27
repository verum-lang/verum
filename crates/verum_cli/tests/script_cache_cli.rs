//! End-to-end CLI tests for `verum cache <subcmd>` (P5.2).
//!
//! These tests populate a custom cache root with synthetic entries via
//! the public `verum_cli::script::cache::ScriptCache` API, then drive the
//! `verum cache` subcommands as a subprocess and assert on stdout. The
//! cache root is overridden via `--root <DIR>` for every subcommand so
//! the user's real `$HOME/.verum/script-cache` is never touched.

use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;
use verum_cli::script::cache::{key_for, ScriptCache};

fn run_cache(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_verum"))
        .arg("cache")
        .args(args)
        .output()
        .expect("verum cache")
}

fn populate(root: &Path, n: u64) {
    let cache = ScriptCache::at(root.to_path_buf()).expect("cache");
    for i in 0..n {
        let src = format!("// entry {i}\nfn main() -> Int {{ {i} }}\n");
        let key = key_for(src.as_bytes(), "0.1.0", &[]);
        cache
            .store(
                key,
                src.as_bytes(),
                format!("/tmp/script_{i}.vr"),
                src.len() as u64,
                "0.1.0",
            )
            .expect("store");
    }
}

#[test]
fn cache_path_prints_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("script-cache");
    let _ = ScriptCache::at(root.clone()).unwrap();
    let out = run_cache(&["path", "--root", root.to_str().unwrap()]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim().ends_with("script-cache"),
        "expected path to end in script-cache, got {stdout:?}"
    );
}

#[test]
fn cache_list_empty_says_so() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    let _ = ScriptCache::at(root.clone()).unwrap();
    let out = run_cache(&["list", "--root", root.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("cache empty"), "got {stdout:?}");
}

#[test]
fn cache_list_table_includes_entries() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 3);
    let out = run_cache(&["list", "--root", root.to_str().unwrap()]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("entries    : 3"), "got {stdout:?}");
    assert!(stdout.contains("KEY"), "table header missing: {stdout:?}");
    assert!(stdout.contains("script_0.vr"), "got {stdout:?}");
    assert!(stdout.contains("script_1.vr"));
    assert!(stdout.contains("script_2.vr"));
}

#[test]
fn cache_list_json_emits_ndjson() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 2);
    let out = run_cache(&["list", "--root", root.to_str().unwrap(), "--json"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 ndjson lines, got {stdout:?}");
    for line in &lines {
        assert!(line.starts_with('{') && line.ends_with('}'), "{line:?}");
        assert!(line.contains(r#""key":""#));
        assert!(line.contains(r#""vbc_len":"#));
    }
}

#[test]
fn cache_list_rejects_bad_sort() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 1);
    let out = run_cache(&["list", "--root", root.to_str().unwrap(), "--sort", "bogus"]);
    assert!(!out.status.success(), "must fail on invalid --sort");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--sort"), "got stderr={stderr:?}");
}

#[test]
fn cache_clear_removes_all_entries_with_yes_flag() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 4);
    let out = run_cache(&["clear", "--root", root.to_str().unwrap(), "--yes"]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("removed 4 entries"));

    // Subsequent list must be empty.
    let out = run_cache(&["list", "--root", root.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("cache empty"));
}

#[test]
fn cache_clear_on_empty_short_circuits() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    let _ = ScriptCache::at(root.clone()).unwrap();
    let out = run_cache(&["clear", "--root", root.to_str().unwrap(), "--yes"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("already empty"));
}

#[test]
fn cache_gc_dry_run_evicts_nothing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 5);
    let before = ScriptCache::at(root.clone()).unwrap().list().unwrap().len();
    let out = run_cache(&[
        "gc",
        "--root",
        root.to_str().unwrap(),
        "--max-size",
        "0",
        "--dry-run",
    ]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("dry-run"), "got {stdout:?}");
    let after = ScriptCache::at(root.clone()).unwrap().list().unwrap().len();
    assert_eq!(before, after, "dry-run must not modify the cache");
}

#[test]
fn cache_gc_evicts_to_target_size() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 5);
    let out = run_cache(&[
        "gc",
        "--root",
        root.to_str().unwrap(),
        "--max-size",
        "0",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("evicted"), "got {stdout:?}");
    let remaining = ScriptCache::at(root).unwrap().list().unwrap().len();
    assert_eq!(remaining, 0, "max-size 0 must evict everything");
}

#[test]
fn cache_gc_rejects_bad_size() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    let _ = ScriptCache::at(root.clone()).unwrap();
    let out = run_cache(&[
        "gc",
        "--root",
        root.to_str().unwrap(),
        "--max-size",
        "12X",
    ]);
    assert!(!out.status.success(), "must fail on bad size");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--max-size"), "got stderr={stderr:?}");
}

#[test]
fn cache_show_finds_unique_prefix() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 1);
    let cache = ScriptCache::at(root.clone()).unwrap();
    let entries = cache.list().unwrap();
    let key = entries[0].0;
    let hex = key.to_hex();
    let prefix = &hex[..12];
    let out = run_cache(&["show", "--root", root.to_str().unwrap(), prefix]);
    assert!(out.status.success(), "stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&hex), "full key not echoed: {stdout:?}");
    assert!(stdout.contains("source_path"));
    assert!(stdout.contains("compiler_version"));
    assert!(stdout.contains("script_0.vr"));
}

#[test]
fn cache_show_rejects_short_prefix() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 1);
    let out = run_cache(&["show", "--root", root.to_str().unwrap(), "ab"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("at least 4"), "got stderr={stderr:?}");
}

#[test]
fn cache_show_no_match_errors() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("c");
    populate(&root, 1);
    let out = run_cache(&[
        "show",
        "--root",
        root.to_str().unwrap(),
        "ffffffffffff",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no cache entry"), "got stderr={stderr:?}");
}
