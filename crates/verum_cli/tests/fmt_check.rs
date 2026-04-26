//! `verum fmt --check` exit-code contract.
//!
//! Pre-commit hooks and CI gates rely on this: exit 0 when every
//! file is canonically formatted, non-zero otherwise. The exit code
//! is the only signal a script consumes — locking it down is just
//! as important as locking down the format itself.

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

fn make_fixture(name: &str, body: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("verum_fmt_check_{}_{}", name, std::process::id()));
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
fn check_exits_zero_on_canonical_input() {
    // The canonical form of `fn main() {}` already has the trailing
    // newline + zero whitespace deviations that fmt would produce.
    let dir = make_fixture("clean", "fn main() {}\n");
    let out = Command::new(binary())
        .args(["fmt", "--check"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt --check failed to spawn");
    assert!(
        out.status.success(),
        "verum fmt --check should exit 0 on canonical input. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_exits_nonzero_on_dirty_input() {
    // A file with extra trailing blanks needs reformatting.
    let dir = make_fixture("dirty", "fn main() {}\n\n\n\n");
    let out = Command::new(binary())
        .args(["fmt", "--check"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt --check failed to spawn");
    assert!(
        !out.status.success(),
        "verum fmt --check should exit non-zero on dirty input. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_writes_canonical_form() {
    // Without --check, fmt should rewrite the file in place.
    let dir = make_fixture("write", "fn main() {}\n\n\n\n");
    let path = dir.join("src").join("main.vr");

    let out = Command::new(binary())
        .args(["fmt"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt failed to spawn");
    assert!(
        out.status.success(),
        "verum fmt should exit 0 on a writable corpus. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let after = std::fs::read_to_string(&path).expect("read fixture");
    assert!(
        !after.ends_with("\n\n\n\n"),
        "trailing-blank stack should have been collapsed, got: {after:?}"
    );

    // Re-running --check after the rewrite must succeed (idempotent).
    let out2 = Command::new(binary())
        .args(["fmt", "--check"])
        .current_dir(&dir)
        .output()
        .expect("verum fmt --check failed to spawn");
    assert!(
        out2.status.success(),
        "verum fmt --check after a clean run should exit 0. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out2.stdout),
        String::from_utf8_lossy(&out2.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
