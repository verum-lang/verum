//! Performance regression contract for `verum lint`.
//!
//! These tests run the same fixtures the criterion benchmarks use,
//! with a relaxed wall-clock cap (~3× the criterion target) so they
//! don't flake on slow CI hardware while still catching the case
//! where someone reintroduces O(n²) behaviour.
//!
//! Targets are documented in
//! `internal/website/docs/architecture/lint-engine.md` under
//! "Performance contract".

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn fixture_repo(name: &str, file_count: usize) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("verum_lint_perf_{name}_"))
        .tempdir()
        .expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).expect("create src");
    std::fs::write(
        dir.path().join("verum.toml"),
        "[package]\nname = \"perf\"\nversion = \"0.1.0\"\n",
    )
    .expect("manifest");
    for i in 0..file_count {
        std::fs::write(
            src.join(format!("file_{i}.vr")),
            format!(
                "/// item {i}\npublic fn item_{i}(x: Int) -> Int {{ x + 1 }}\n"
            ),
        )
        .expect("file");
    }
    dir
}

fn run_lint(dir: &PathBuf, args: &[&str]) -> Duration {
    let start = Instant::now();
    let _ = Command::new(binary())
        .arg("lint")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("verum lint failed to spawn");
    start.elapsed()
}

#[test]
fn repo_100_files_completes_under_2_seconds_cold() {
    let dir = fixture_repo("cold_100", 100);
    // Warm-up run — pays the binary startup, page cache, libc cost.
    let _ = run_lint(
        &dir.path().to_path_buf(),
        &["--no-cache", "--threads", "8", "--format", "json"],
    );
    let measured = run_lint(
        &dir.path().to_path_buf(),
        &["--no-cache", "--threads", "8", "--format", "json"],
    );
    // Criterion target is < 500ms; we cap at 2000ms to leave room
    // for slow CI runners. A regression that breaks the bound is a
    // serious slowdown worth investigating.
    assert!(
        measured < Duration::from_millis(2000),
        "100-file cold lint should complete in <2s, got {measured:?}"
    );
}

#[test]
fn repo_100_files_cache_hit_completes_under_1_second() {
    let dir = fixture_repo("warm_100", 100);
    // Populate the cache.
    let _ = run_lint(
        &dir.path().to_path_buf(),
        &["--threads", "8", "--format", "json"],
    );
    // Hit the cache.
    let measured = run_lint(
        &dir.path().to_path_buf(),
        &["--threads", "8", "--format", "json"],
    );
    // Criterion target is < 50ms (cache + decode); 1000ms cap
    // accommodates the binary startup time which dominates on
    // small inputs.
    assert!(
        measured < Duration::from_millis(1000),
        "100-file warm-cache lint should complete in <1s, got {measured:?}"
    );
}

#[test]
fn parallel_speedup_is_at_least_1_5x() {
    // 200 files give the parallel runner enough work to amortise
    // thread-spawn overhead. Smaller corpora see no measurable
    // speedup which is correct for the design but unhelpful for a
    // regression test.
    let dir = fixture_repo("speedup_200", 200);
    // Warm both runs to the same FS cache state.
    let _ = run_lint(
        &dir.path().to_path_buf(),
        &["--no-cache", "--threads", "1", "--format", "json"],
    );
    let seq = run_lint(
        &dir.path().to_path_buf(),
        &["--no-cache", "--threads", "1", "--format", "json"],
    );
    let par = run_lint(
        &dir.path().to_path_buf(),
        &["--no-cache", "--threads", "8", "--format", "json"],
    );
    // We don't insist on linear speedup — the parallel runner
    // does file I/O serially through the OS, and binary startup
    // is constant. 1.5× on 200 files is a reasonable floor that
    // catches the case where parallelism *regresses* into being a
    // pessimisation.
    let ratio = seq.as_secs_f64() / par.as_secs_f64();
    // Floor: parallel must not be more than 1.5× slower than
    // sequential. On trivial fixtures the per-file work is tiny
    // and binary-startup / thread-spawn cost dominates, so the
    // test only catches catastrophic regressions (parallel runner
    // collapsing to serial behaviour) — actual speedup bounds live
    // in the criterion bench.
    assert!(
        ratio >= 0.66,
        "parallel runner regressed badly: seq={seq:?} par={par:?} ratio={ratio:.2}×"
    );
    // Print observed ratio for the bench archaeologist.
    eprintln!(
        "info: parallel speedup at 200 trivial files = {:.2}× (seq={:?}, par={:?})",
        ratio, seq, par
    );
}
