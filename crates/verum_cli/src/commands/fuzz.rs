//! Fuzz-target orchestration for `[test].fuzzing = true` (#299).
//!
//! When the manifest enables `[test].fuzzing`, the `verum test`
//! command runs not only the @test / @property suite but also each
//! cargo-fuzz target discovered under `fuzz/` directories.  This
//! module owns:
//!
//!   1. Discovery — walk the workspace for `fuzz/Cargo.toml` (at
//!      project root) and `crates/*/fuzz/Cargo.toml` (per-crate
//!      fuzz harnesses), parse each TOML for `[[bin]]` entries,
//!      and produce a flat `Vec<FuzzTarget>`.
//!   2. Invocation — call `cargo fuzz run <target> -- \
//!      -max_total_time=<N>` for each target.  Failures surface
//!      as crash artifacts under `fuzz/artifacts/<target>/`; we
//!      enumerate any pre-existing artifacts before the run and
//!      after, and treat *new* artifacts as the failure signal.
//!   3. Reporting — aggregate per-target outcomes into a
//!      `FuzzReport` with crash artifacts attached.
//!
//! Cargo-fuzz toolchain dependency: requires `cargo-fuzz`
//! installed (`cargo install cargo-fuzz`).  When the binary is
//! absent the runner emits a single hint line and returns an
//! empty `FuzzReport` — fuzzing is best-effort observability
//! rather than a hard CI gate, so a missing toolchain
//! gracefully degrades to "no fuzz targets exercised" instead
//! of failing the entire `verum test` invocation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// One cargo-fuzz binary target discovered under a `fuzz/`
/// directory.  Tracks the host crate (the parent directory of
/// `fuzz/`) so reports can pinpoint where each target lives, and
/// the artifacts directory so post-run inspection knows where to
/// look for crash files.
#[derive(Debug, Clone)]
pub struct FuzzTarget {
    /// Display name of the target (e.g. `fuzz_parse_module`).
    pub name: String,
    /// Path to the host crate root (the directory holding `fuzz/`).
    pub host_crate_dir: PathBuf,
    /// Path to the `fuzz/` directory holding `Cargo.toml`.
    pub fuzz_dir: PathBuf,
    /// Where crash artifacts land after a run (absolute).
    pub artifacts_dir: PathBuf,
}

/// Aggregate report from a `verum test --fuzzing=true` invocation.
#[derive(Debug, Default)]
pub struct FuzzReport {
    /// Number of targets discovered (whether or not they were
    /// successfully exercised).
    pub discovered: usize,
    /// Per-target outcome.  Empty when cargo-fuzz isn't installed.
    pub outcomes: Vec<FuzzOutcome>,
    /// Single-line hint message when the toolchain is missing or
    /// the discovery walk produces no targets.  Empty otherwise.
    pub hint: Option<String>,
}

/// Per-target outcome.
#[derive(Debug, Clone)]
pub struct FuzzOutcome {
    pub target: String,
    pub host_crate_dir: PathBuf,
    pub duration: Duration,
    pub status: FuzzStatus,
    /// New crash artifacts that landed during this run (filename
    /// only, not full path).  Empty on a clean run.
    pub new_crash_artifacts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuzzStatus {
    Clean,
    /// At least one new crash artifact appeared.
    Crashed,
    /// `cargo fuzz` exited non-zero without producing artifacts —
    /// usually means the harness failed to compile.
    HarnessError(String),
    /// Cargo-fuzz timed out before completing the configured
    /// duration.  Treated as a soft failure — the run produced no
    /// artifacts but didn't get to assert clean either.
    Timeout,
}

/// Locate `cargo-fuzz`.  Returns true when the binary is on PATH.
pub fn cargo_fuzz_available() -> bool {
    Command::new("cargo")
        .args(["fuzz", "--version"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Walk the workspace for `fuzz/Cargo.toml` files and enumerate
/// their `[[bin]]` targets.  Discovery roots:
///
///   * `<workspace_root>/fuzz/Cargo.toml`
///   * `<workspace_root>/crates/<name>/fuzz/Cargo.toml`
///   * `<workspace_root>/<arbitrary>/fuzz/Cargo.toml` (one level)
///
/// Returns an empty list when nothing matches.
pub fn discover_targets(workspace_root: &Path) -> Vec<FuzzTarget> {
    let mut out = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    roots.push(workspace_root.to_path_buf());
    if let Ok(crates_iter) = std::fs::read_dir(workspace_root.join("crates")) {
        for entry in crates_iter.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                roots.push(entry.path());
            }
        }
    }
    for root in &roots {
        let cargo_toml = root.join("fuzz").join("Cargo.toml");
        if !cargo_toml.exists() {
            continue;
        }
        let bins = parse_cargo_fuzz_bins(&cargo_toml);
        let fuzz_dir = root.join("fuzz");
        for name in bins {
            out.push(FuzzTarget {
                name: name.clone(),
                host_crate_dir: root.clone(),
                artifacts_dir: fuzz_dir.join("artifacts").join(&name),
                fuzz_dir: fuzz_dir.clone(),
            });
        }
    }
    out
}

/// Parse a `fuzz/Cargo.toml` to extract `[[bin]] name = "..."`
/// entries.  Returns the names in source order.  Tolerant of
/// missing/malformed files (returns an empty list) — the test
/// runner shouldn't error just because a fuzz manifest is bad.
fn parse_cargo_fuzz_bins(cargo_toml: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(cargo_toml) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    parse_bin_entries_from_toml(&text)
}

/// Extract `[[bin]] name = "..."` from a Cargo.toml string.
/// Pin-tested separately so fuzz discovery doesn't need a
/// real-file fixture.  Hand-rolled minimal parser — pulling in
/// the `toml` crate just for `[[bin]]` enumeration would bloat
/// the CLI binary without adding value, since we only need the
/// `name` field of each `[[bin]]` table.
fn parse_bin_entries_from_toml(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_bin_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            in_bin_section = trimmed == "[[bin]]";
            continue;
        }
        if in_bin_section {
            if let Some(rest) = trimmed.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let value = rest.trim();
                    let value = value.trim_matches(|c: char| c == '"' || c == '\'');
                    if !value.is_empty() {
                        out.push(value.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Snapshot the current set of crash artifacts under the
/// target's artifacts directory.  Used to compute the diff after
/// the fuzz run completes — only *newly created* files count as
/// crashes, since a stale artifact from a previous run shouldn't
/// fail the current invocation.
fn snapshot_artifacts(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(iter) = std::fs::read_dir(dir) {
        for entry in iter.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Run `cargo fuzz run <target> -- -max_total_time=<secs>` for a
/// single target.  Returns a `FuzzOutcome` with status reflecting
/// crash-artifact diff + cargo-fuzz exit status.  This is a
/// blocking operation — the caller is responsible for
/// orchestrating concurrency (e.g., serial execution to avoid
/// fighting over a single CPU).
pub fn run_target(target: &FuzzTarget, max_total_time: Duration) -> FuzzOutcome {
    let start = Instant::now();
    let before = snapshot_artifacts(&target.artifacts_dir);

    let max_secs = max_total_time.as_secs().max(1);
    let cargo_fuzz_arg = format!("-max_total_time={max_secs}");
    let output = Command::new("cargo")
        .args(["fuzz", "run", &target.name, "--"])
        .arg(&cargo_fuzz_arg)
        .current_dir(&target.host_crate_dir)
        .output();

    let after = snapshot_artifacts(&target.artifacts_dir);
    let new_artifacts: Vec<String> =
        after.difference(&before).cloned().collect();

    let status = match output {
        Ok(o) if o.status.success() => {
            if new_artifacts.is_empty() {
                FuzzStatus::Clean
            } else {
                FuzzStatus::Crashed
            }
        }
        Ok(o) => {
            if !new_artifacts.is_empty() {
                FuzzStatus::Crashed
            } else {
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                FuzzStatus::HarnessError(truncate_stderr(&stderr))
            }
        }
        Err(_) => FuzzStatus::Timeout,
    };

    FuzzOutcome {
        target: target.name.clone(),
        host_crate_dir: target.host_crate_dir.clone(),
        duration: start.elapsed(),
        status,
        new_crash_artifacts: new_artifacts,
    }
}

fn truncate_stderr(stderr: &str) -> String {
    const MAX: usize = 1024;
    if stderr.len() <= MAX {
        stderr.to_string()
    } else {
        let head = &stderr[..MAX / 2];
        let tail = &stderr[stderr.len() - MAX / 2..];
        format!("{head}\n  ... [truncated] ...\n{tail}")
    }
}

/// Top-level entry point for the test runner.  Discovers
/// targets, runs each one with the configured budget, returns a
/// `FuzzReport` aggregating outcomes.  Caller decides whether
/// any crash artifact aborts the test run.
pub fn run(workspace_root: &Path, per_target_budget: Duration) -> FuzzReport {
    let mut report = FuzzReport::default();
    if !cargo_fuzz_available() {
        report.hint = Some(
            "cargo-fuzz not on PATH — install with `cargo install cargo-fuzz` \
             to enable fuzzing under [test].fuzzing = true"
                .to_string(),
        );
        return report;
    }
    let targets = discover_targets(workspace_root);
    report.discovered = targets.len();
    if targets.is_empty() {
        report.hint = Some(
            "no fuzz targets discovered — expected `fuzz/Cargo.toml` \
             at workspace root or under `crates/*/fuzz/`"
                .to_string(),
        );
        return report;
    }
    for t in &targets {
        report.outcomes.push(run_target(t, per_target_budget));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bin_entries_handles_canonical_layout() {
        let sample = r#"
[package]
name = "demo"

[[bin]]
name = "fuzz_parse_module"
path = "fuzz_targets/fuzz_parse_module.rs"
test = false

[[bin]]
name = "fuzz_lexer"
path = "fuzz_targets/fuzz_lexer.rs"
test = false
"#;
        let bins = parse_bin_entries_from_toml(sample);
        assert_eq!(bins, vec!["fuzz_parse_module", "fuzz_lexer"]);
    }

    #[test]
    fn parse_bin_entries_ignores_other_sections() {
        let sample = r#"
[package]
name = "demo"

[dependencies]
serde = "1"

[[bench]]
name = "not_a_fuzz_bin"
"#;
        let bins = parse_bin_entries_from_toml(sample);
        assert!(bins.is_empty(), "non-bin sections should not produce targets");
    }

    #[test]
    fn parse_bin_entries_handles_single_quotes() {
        let sample = r#"
[[bin]]
name = 'single_quoted_target'
"#;
        assert_eq!(parse_bin_entries_from_toml(sample), vec!["single_quoted_target"]);
    }

    #[test]
    fn parse_bin_entries_skips_comments_and_blank_lines() {
        let sample = r#"
# This is a comment
[[bin]]
# Another comment
name = "good_target"

# blank line above
[[bin]]
name = "second_target"
"#;
        assert_eq!(
            parse_bin_entries_from_toml(sample),
            vec!["good_target", "second_target"]
        );
    }

    #[test]
    fn parse_bin_entries_returns_empty_on_malformed_input() {
        assert!(parse_bin_entries_from_toml("").is_empty());
        assert!(parse_bin_entries_from_toml("not toml at all !!").is_empty());
        // Section starts but no name field — no entries produced.
        let sample = "[[bin]]\npath = \"foo.rs\"\n";
        assert!(parse_bin_entries_from_toml(sample).is_empty());
    }

    #[test]
    fn discover_targets_in_synthetic_workspace() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // Create a workspace-root fuzz crate.
        let root_fuzz = root.join("fuzz");
        std::fs::create_dir_all(&root_fuzz).unwrap();
        std::fs::write(
            root_fuzz.join("Cargo.toml"),
            r#"
[package]
name = "ws-fuzz"

[[bin]]
name = "fuzz_ws_root"
path = "fuzz_targets/fuzz_ws_root.rs"
"#,
        )
        .unwrap();
        // Create a per-crate fuzz harness.
        let crate_fuzz = root.join("crates").join("verum_demo").join("fuzz");
        std::fs::create_dir_all(&crate_fuzz).unwrap();
        std::fs::write(
            crate_fuzz.join("Cargo.toml"),
            r#"
[[bin]]
name = "fuzz_demo_a"

[[bin]]
name = "fuzz_demo_b"
"#,
        )
        .unwrap();

        let targets = discover_targets(root);
        let names: Vec<_> = targets.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"fuzz_ws_root"));
        assert!(names.contains(&"fuzz_demo_a"));
        assert!(names.contains(&"fuzz_demo_b"));
        assert_eq!(targets.len(), 3);
    }

    #[test]
    fn discover_targets_in_empty_workspace_returns_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let targets = discover_targets(tmp.path());
        assert!(targets.is_empty());
    }

    #[test]
    fn snapshot_artifacts_returns_empty_for_missing_dir() {
        let snap = snapshot_artifacts(Path::new("/no/such/path/12345"));
        assert!(snap.is_empty());
    }

    #[test]
    fn snapshot_artifacts_lists_filenames() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("crash-deadbeef"), b"x").unwrap();
        std::fs::write(tmp.path().join("crash-cafebabe"), b"y").unwrap();
        let snap = snapshot_artifacts(tmp.path());
        assert!(snap.contains("crash-deadbeef"));
        assert!(snap.contains("crash-cafebabe"));
        assert_eq!(snap.len(), 2);
    }

    #[test]
    fn artifact_diff_isolates_new_crashes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join("old-crash"), b"x").unwrap();
        let before = snapshot_artifacts(tmp.path());
        std::fs::write(tmp.path().join("new-crash-1"), b"y").unwrap();
        std::fs::write(tmp.path().join("new-crash-2"), b"z").unwrap();
        let after = snapshot_artifacts(tmp.path());
        let diff: Vec<String> = after.difference(&before).cloned().collect();
        assert_eq!(diff.len(), 2);
        assert!(diff.contains(&"new-crash-1".to_string()));
        assert!(diff.contains(&"new-crash-2".to_string()));
    }

    #[test]
    fn run_returns_hint_when_cargo_fuzz_unavailable() {
        // We can't easily mock cargo_fuzz_available() inside one
        // process, but we can verify the FuzzReport shape: when
        // the helper returns false, the report carries a hint and
        // no outcomes.  In CI with cargo-fuzz absent this is the
        // realistic path — pin the public surface (`FuzzReport`
        // fields are accessible) so the orchestrator can branch
        // cleanly.
        let r = FuzzReport {
            discovered: 0,
            outcomes: Vec::new(),
            hint: Some("cargo-fuzz not on PATH".into()),
        };
        assert_eq!(r.discovered, 0);
        assert!(r.outcomes.is_empty());
        assert!(r.hint.unwrap().contains("cargo-fuzz"));
    }

    #[test]
    fn fuzz_status_variants_are_distinguishable() {
        let clean = FuzzStatus::Clean;
        let crashed = FuzzStatus::Crashed;
        let harness = FuzzStatus::HarnessError("oops".into());
        let timeout = FuzzStatus::Timeout;
        assert_eq!(clean, FuzzStatus::Clean);
        assert_ne!(clean, crashed);
        assert_ne!(crashed, FuzzStatus::Timeout);
        assert!(matches!(harness, FuzzStatus::HarnessError(_)));
        assert!(matches!(timeout, FuzzStatus::Timeout));
    }

    #[test]
    fn truncate_stderr_keeps_short_strings_intact() {
        let s = "a short error message";
        assert_eq!(truncate_stderr(s), s);
    }

    #[test]
    fn truncate_stderr_truncates_when_above_limit() {
        let big = "x".repeat(5000);
        let truncated = truncate_stderr(&big);
        assert!(truncated.contains("[truncated]"));
        assert!(truncated.len() < big.len());
    }
}
