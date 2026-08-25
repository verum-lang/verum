//! T0867 — every program in `docs/by-example/` must type-check.
//!
//! The corpus is the language's own showcase: 22 numbered programs, one
//! per feature area, each written to be read AND run. Nothing ran them.
//! Measured 2026-08-25 before this gate existed: **9 of 22 failed** — 3
//! could not be parsed at all, 6 failed later. The roots split evenly
//! between genuine compiler defects (a refinement hiding the base type's
//! indexability, `Int.MAX` invisible to the verifier, tiered receivers
//! demoted to associated functions, `extern` symbols unresolvable on
//! macOS) and examples written against an API that never existed
//! (`to_int` where `parse_int` was meant, `Duration.from_secs` mounted as
//! a free function, a Rust turbofish, single quotes around a word).
//!
//! Documentation that does not run rots silently and takes the reader's
//! trust with it, so the corpus is now measured rather than assumed.
//!
//! **Where this runs**: `verum_compiler`'s suites are on the nightly AOT
//! lane, not the PR gate (see the repo's CI notes). It is a measurement
//! lane, not a blocker — run it locally before touching either the
//! corpus or name resolution.

use std::path::{Path, PathBuf};
use verum_compiler::{CompilationPipeline, CompilerOptions, Session};

/// The corpus is expected to keep growing; this is the floor as of the
/// day the gate landed. It exists so that a corpus that vanishes — a
/// moved directory, a bad glob — fails LOUDLY instead of reporting a
/// green run over zero files.
const MINIMUM_EXAMPLES: usize = 20;

fn corpus_root() -> PathBuf {
    // tests/ lives at crates/verum_compiler/tests, so the repo root is
    // three levels up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("by-example")
}

fn examples() -> Vec<PathBuf> {
    let root = corpus_root();
    let mut found: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("by-example corpus unreadable at {}: {e}", root.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("main.vr"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

fn label(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Type-check one example, returning the failure text if it did not pass.
fn check_failure(path: &Path) -> Option<String> {
    let options = CompilerOptions {
        input: path.to_path_buf(),
        output: std::env::temp_dir().join("verum-by-example-gate"),
        ..Default::default()
    };
    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);
    match pipeline.run_check_only() {
        Ok(()) => None,
        Err(e) => Some(format!("{e}")),
    }
}

#[test]
fn the_corpus_exists_and_is_not_empty() {
    // The control for every assertion below. Without it, a gate whose
    // glob stops matching reports success over an empty list — the
    // most expensive kind of green.
    let found = examples();
    assert!(
        found.len() >= MINIMUM_EXAMPLES,
        "expected at least {MINIMUM_EXAMPLES} by-example programs under {}, found {}: {:?}",
        corpus_root().display(),
        found.len(),
        found.iter().map(|p| label(p)).collect::<Vec<_>>()
    );
}

#[test]
fn every_by_example_program_type_checks() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let found = examples();
            assert!(
                found.len() >= MINIMUM_EXAMPLES,
                "corpus shrank to {} files — see the_corpus_exists_and_is_not_empty",
                found.len()
            );

            let mut failures: Vec<(String, String)> = Vec::new();
            for path in &found {
                if let Some(err) = check_failure(path) {
                    failures.push((label(path), err));
                }
            }

            assert!(
                failures.is_empty(),
                "{} of {} by-example programs do not type-check:\n{}",
                failures.len(),
                found.len(),
                failures
                    .iter()
                    .map(|(name, err)| format!("  {name}: {}", err.lines().next().unwrap_or(err)))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
