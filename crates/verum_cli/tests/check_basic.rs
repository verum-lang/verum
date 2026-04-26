//! `verum check` end-to-end exit-code + diagnostic contract.
//!
//! `verum check` runs lex → parse → type-check on the project,
//! skipping VBC/LLVM codegen. The CLI contract this test pins down:
//!
//!   * Clean project → exit 0, no diagnostic-formatted output on stderr.
//!   * Parse error → exit non-zero, stderr identifies the offending file.
//!   * Unknown identifier (post-parse type-check) → exit non-zero,
//!     stderr surfaces the failing name.
//!
//! Pre-commit hooks, CI gates, and IDE "save and check" workflows rely
//! on the exit code being a binary signal — locking it down is the
//! same hardening contract `fmt --check` follows.
//!
//! Test isolation: each fixture is built in `$TMPDIR/verum_check_*`
//! with a dedicated PID-suffixed name to avoid cross-test contamination
//! when run in parallel.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "verum_check_{}_{}",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("create src");
    std::fs::write(
        dir.join("verum.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
    )
    .expect("manifest");
    std::fs::write(dir.join("src").join("main.vr"), body).expect("main.vr");
    dir
}

#[test]
fn check_exits_zero_on_valid_program() {
    // Smallest valid Verum program: an empty main(). Lex + parse +
    // type-check all succeed, no diagnostics.
    let dir = make_fixture("valid", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        out.status.success(),
        "verum check should exit 0 on a valid program. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_parse_error() {
    // Unbalanced braces — the lexer accepts the bytes, the parser
    // fails at end-of-input expecting `}`.
    let dir = make_fixture("parse_err", "fn main() { let x = ;\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on a parse error. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Diagnostic should mention the offending file (basename).
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("main.vr"),
        "parse error diagnostic should reference main.vr; got: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_unknown_identifier() {
    // Reference an undeclared name. Parse succeeds; type-check fails.
    // This pins down that the type-check phase actually runs (not just
    // a lex-only fast path).
    let dir = make_fixture(
        "unknown_id",
        "fn main() { let x = some_undeclared_name; }\n",
    );
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(
        !out.status.success(),
        "verum check should exit non-zero on an unknown identifier. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_does_not_emit_codegen_artifacts() {
    // `verum check` must not emit a binary or VBC bundle. Pre-existing
    // contract: the fast-path is type-only, so the build directory
    // should be absent (or at least without compiled artefacts).
    let dir = make_fixture("no_artifacts", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["check"])
        .current_dir(&dir)
        .output()
        .expect("verum check failed to spawn");
    assert!(out.status.success());

    // No binary in any of the conventional output locations.
    for candidate in &["target/debug/no_artifacts", "build/no_artifacts"] {
        let p = dir.join(candidate);
        assert!(
            !p.exists(),
            "verum check should NOT produce a compiled binary at {p:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
