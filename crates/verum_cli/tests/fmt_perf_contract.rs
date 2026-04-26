//! Wall-clock regression contract for `verum fmt`. Runs the same
//! fixtures the criterion bench uses with relaxed caps (~3× the
//! criterion target) so CI catches catastrophic regressions without
//! flaking on slow runners.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn fixture_repo(name: &str, file_count: usize) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("verum_fmt_perf_{name}_"))
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
            format!("public fn item_{i}(x: Int) -> Int {{ x + 1 }}\n"),
        )
        .expect("file");
    }
    dir
}

fn run(dir: &PathBuf, args: &[&str]) -> Duration {
    let start = Instant::now();
    let _ = Command::new(binary())
        .arg("fmt")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("verum fmt spawn");
    start.elapsed()
}

#[test]
fn repo_100_check_completes_under_2_seconds() {
    let dir = fixture_repo("check_100", 100);
    // Warm-up — pays binary startup, FS cache.
    let _ = run(&dir.path().to_path_buf(), &["--check", "--threads", "8"]);
    let measured = run(&dir.path().to_path_buf(), &["--check", "--threads", "8"]);
    assert!(
        measured < Duration::from_millis(2000),
        "100-file fmt --check should complete in <2s, got {measured:?}"
    );
}

#[test]
fn repo_100_rewrite_completes_under_2_seconds() {
    let dir = fixture_repo("rewrite_100", 100);
    let _ = run(&dir.path().to_path_buf(), &["--threads", "8"]);
    // Re-run — files are now canonical, so this should be fast
    // (no rewrite needed).
    let measured = run(&dir.path().to_path_buf(), &["--threads", "8"]);
    assert!(
        measured < Duration::from_millis(2000),
        "100-file fmt re-run should complete in <2s, got {measured:?}"
    );
}

#[test]
fn parallel_speedup_within_50_percent_of_sequential() {
    let dir = fixture_repo("speedup_200", 200);
    // Warm both runs to the same FS cache state.
    let _ = run(
        &dir.path().to_path_buf(),
        &["--check", "--threads", "1"],
    );
    let seq = run(
        &dir.path().to_path_buf(),
        &["--check", "--threads", "1"],
    );
    let par = run(
        &dir.path().to_path_buf(),
        &["--check", "--threads", "8"],
    );
    let ratio = seq.as_secs_f64() / par.as_secs_f64();
    // Parallel must not be more than 1.5× slower than sequential
    // — a softer floor than "must be faster" because at 200 trivial
    // files the binary-startup cost dominates.
    assert!(
        ratio >= 0.66,
        "parallel runner regressed badly: seq={seq:?} par={par:?} ratio={ratio:.2}×"
    );
    eprintln!(
        "info: fmt parallel speedup at 200 trivial files = {:.2}× (seq={:?}, par={:?})",
        ratio, seq, par
    );
}
