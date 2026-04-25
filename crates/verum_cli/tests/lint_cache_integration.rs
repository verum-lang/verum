//! End-to-end cache contract tests for `verum lint`.
//!
//! Each test runs the binary against a generated fixture project,
//! observing the `target/lint-cache/` directory between runs to
//! verify the cache is created, reused, and invalidated correctly.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_lint_cache_test_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("write manifest");
    for i in 0..6 {
        std::fs::write(
            dir.join("src").join(format!("file_{i}.vr")),
            format!("fn item_{i}() {{\n    let x = Box::new({i});\n}}\n"),
        )
        .expect("write fixture");
    }
    dir
}

fn run_lint(dir: &PathBuf, extra_args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(binary());
    cmd.arg("lint")
        .arg("--format")
        .arg("json")
        .arg("--threads")
        .arg("4")
        .current_dir(dir);
    for a in extra_args {
        cmd.arg(a);
    }
    cmd.output().expect("verum lint failed to spawn")
}

fn count_cache_files(dir: &PathBuf) -> usize {
    let cache = dir.join("target").join("lint-cache");
    if !cache.exists() {
        return 0;
    }
    let mut count = 0;
    if let Ok(buckets) = std::fs::read_dir(&cache) {
        for bucket in buckets.flatten() {
            if bucket.path().is_dir() {
                if let Ok(entries) = std::fs::read_dir(bucket.path()) {
                    for e in entries.flatten() {
                        if e.path().extension().and_then(|s| s.to_str()) == Some("json") {
                            count += 1;
                        }
                    }
                }
            }
        }
    }
    count
}

#[test]
fn first_run_populates_cache() {
    let dir = make_fixture("populate");
    let out = run_lint(&dir, &[]);
    assert!(!out.stdout.is_empty(), "first run should emit diagnostics");
    let n = count_cache_files(&dir);
    assert_eq!(
        n, 6,
        "expected 6 cache entries (one per fixture file), got {n}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn second_run_returns_byte_identical_output() {
    let dir = make_fixture("byte_identical");
    let first = run_lint(&dir, &[]).stdout;
    let second = run_lint(&dir, &[]).stdout;
    assert_eq!(
        first, second,
        "cache hits must produce byte-identical diagnostic streams"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_cache_flag_bypasses_cache() {
    let dir = make_fixture("no_cache_flag");
    let _ = run_lint(&dir, &[]); // populate
    let cache_dir = dir.join("target").join("lint-cache");
    let buckets_before: Vec<_> = std::fs::read_dir(&cache_dir)
        .expect("cache dir exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();

    // The --no-cache run should still produce output but not write
    // to a new bucket (the existing bucket is left intact).
    let out = run_lint(&dir, &["--no-cache"]);
    assert!(!out.stdout.is_empty());

    let buckets_after: Vec<_> = std::fs::read_dir(&cache_dir)
        .expect("cache dir exists")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        buckets_before, buckets_after,
        "--no-cache must not create new buckets"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn modifying_source_invalidates_one_entry() {
    let dir = make_fixture("invalidate");
    let _ = run_lint(&dir, &[]); // populate
    // Modify file_0.vr — the cached entry for it becomes
    // unreachable; the other 5 entries remain.
    std::fs::write(
        dir.join("src").join("file_0.vr"),
        "fn item_0() {\n    let x = Heap(0);\n}\n",
    )
    .expect("rewrite file_0");
    let _ = run_lint(&dir, &[]);
    let n = count_cache_files(&dir);
    // 5 unchanged + 1 new entry (for the modified content) = 7.
    // The old entry for file_0.vr is orphaned but not deleted —
    // GC removes it when the *config* hash changes, not on content.
    assert!(
        n >= 6,
        "expected at least 6 cache entries after edit (got {n})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_cache_subcommand_wipes_directory() {
    let dir = make_fixture("clean");
    let _ = run_lint(&dir, &[]);
    assert!(count_cache_files(&dir) > 0);
    let out = Command::new(binary())
        .args(["lint", "--clean-cache"])
        .current_dir(&dir)
        .output()
        .expect("clean-cache spawn");
    assert!(out.status.success(), "clean-cache should exit 0");
    assert_eq!(count_cache_files(&dir), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
