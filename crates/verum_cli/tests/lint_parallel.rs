//! Parallel-runner determinism contract.
//!
//! `lint_paths_parallel` walks files via rayon and merges issues
//! into a single Vec. The test below proves the output is sorted
//! into a canonical order regardless of the thread interleaving:
//! running the same fixture under thread counts 1, 2, and 8 must
//! produce byte-identical issue streams.
//!
//! This is the regression gate for any future change that touches
//! the parallel path — losing determinism would silently break CI
//! pipelines that diff lint reports across runs.

use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn fixture_dir() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("parallel_corpus");
    p
}

fn ensure_fixture() {
    let dir = fixture_dir();
    let src_dir = dir.join("src");
    if src_dir.exists() {
        return;
    }
    std::fs::create_dir_all(&src_dir).expect("create src/");
    std::fs::write(
        dir.join("verum.toml"),
        "[package]\nname = \"parallel_corpus\"\nversion = \"0.1.0\"\n",
    )
    .expect("write verum.toml");
    for i in 0..40 {
        let body = format!(
            "fn item_{i}() {{\n    // TODO: clean up\n    let x = Box::new({i});\n    let y = Heap({i});\n}}\n"
        );
        std::fs::write(src_dir.join(format!("file_{i}.vr")), body).expect("write fixture");
    }
}

fn run(threads: usize) -> String {
    let dir = fixture_dir();
    let out = Command::new(binary())
        .args([
            "lint",
            "--threads",
            &threads.to_string(),
            "--format",
            "json",
        ])
        .current_dir(&dir)
        .output()
        .expect("verum lint failed to spawn");
    // The binary returns non-zero when lint issues exist. We don't
    // care for this test — we want the JSON output regardless.
    String::from_utf8(out.stdout).expect("stdout is UTF-8")
}

#[test]
fn parallel_output_matches_sequential() {
    ensure_fixture();
    let seq = run(1);
    let par = run(8);
    assert_eq!(
        seq, par,
        "lint output must be deterministic across thread counts"
    );
    assert!(!seq.is_empty(), "fixture should produce diagnostics");
}

#[test]
fn parallel_output_matches_under_two_threads() {
    ensure_fixture();
    let two = run(2);
    let four = run(4);
    assert_eq!(two, four, "lint output must be deterministic at 2 vs 4 threads");
}

#[test]
fn lint_emits_issues_for_every_fixture_file() {
    ensure_fixture();
    let out = run(4);
    // 40 files × multiple rules each → at least 40 lines.
    let line_count = out.lines().count();
    assert!(
        line_count >= 40,
        "expected at least 40 issue lines, got {}",
        line_count
    );
}
