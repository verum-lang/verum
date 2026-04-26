//! Parallel-runner determinism contract for `verum fmt`.
//!
//! 1-thread vs 8-thread runs against the same fixture must rewrite
//! every file in exactly the same way. The contract is that
//! parallelism never mutates the *output*, only the *wall-clock
//! time it takes to produce*.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, file_count: usize) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_fmt_par_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    for i in 0..file_count {
        std::fs::write(
            dir.join("src").join(format!("file_{i}.vr")),
            format!(
                "fn item_{i}() {{\n    let x = {i};\n}}\n\n\n\n\nfn item_{i}_b() {{}}\n"
            ),
        )
        .expect("file");
    }
    dir
}

fn run_check(dir: &PathBuf, threads: usize) -> std::process::Output {
    Command::new(binary())
        .args([
            "fmt",
            "--check",
            "--verbose",
            "--threads",
            &threads.to_string(),
        ])
        .current_dir(dir)
        .output()
        .expect("verum fmt --check spawn")
}

#[test]
fn parallel_check_output_matches_sequential() {
    let dir = make_fixture("determinism", 30);
    let seq = run_check(&dir, 1);
    let par = run_check(&dir, 8);
    assert_eq!(
        seq.status.code(),
        par.status.code(),
        "exit codes must match"
    );
    assert_eq!(
        seq.stdout, par.stdout,
        "stdout must be byte-identical between sequential and parallel runs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_rewrites_every_dirty_file_in_parallel() {
    let dir = make_fixture("rewrite", 20);
    let out = Command::new(binary())
        .args(["fmt", "--threads", "8"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt spawn");
    assert!(out.status.success(), "verum fmt should exit 0");

    // After the run every file must already be canonically
    // formatted — i.e. --check exits 0.
    let check = Command::new(binary())
        .args(["fmt", "--check", "--threads", "8"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt --check spawn");
    assert!(
        check.status.success(),
        "after fmt run, --check should exit 0. stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
