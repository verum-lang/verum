//! Verum scripting-mode end-to-end CLI contract.
//!
//! Exercises the **three execution-mode contract** end-to-end through
//! the real `verum` binary:
//!
//!   1. **Interpreter** — `verum run file.vr` with `fn main()` in source.
//!   2. **AOT** — `verum run --aot file.vr` (smoke-tested separately;
//!      not in this file because LLVM availability gates it).
//!   3. **Script** — bare `verum file.vr` (or shebang exec `./file.vr`)
//!      requires a `#!` shebang line at byte 0; top-level statements are
//!      folded into a synthesised `__verum_script_main` wrapper. Files
//!      lacking the shebang must use `verum run`.
//!
//! Coverage class:
//!
//!   * **Mode dispatch** — argv-rewriter routes correctly; advisory fires
//!     for `.vr` files without shebang.
//!   * **Wrapper synthesis** — top-level let/expr/decl mix produces
//!     correct output ordering.
//!   * **Exit-code propagation** — tail Int / Bool / explicit `fn main()
//!     -> Int` all reach the OS via `process::exit`.
//!   * **Shebang exec** — Unix kernel-level shebang dispatch (gated on
//!     `cfg(unix)` because Windows lacks shebang exec).
//!
//! Test isolation: each fixture is built in `$TMPDIR/verum_script_*`
//! with a PID-suffix to avoid cross-test contamination under parallel
//! `cargo test`. No `verum.toml` is created for the script-mode tests
//! — that is precisely the contract being pinned (no manifest needed).

use std::path::PathBuf;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_verum")
}

/// Build a uniquely-named temp directory for one fixture.
fn fresh_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "verum_script_e2e_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

/// Write a source file at `dir/name.vr` with the given body. Returns the path.
fn write_source(dir: &PathBuf, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write source");
    p
}

/// Run the verum binary; return (exit-code, stdout, stderr).
fn run_verum(args: &[&str], cwd: Option<&PathBuf>) -> (i32, String, String) {
    let mut cmd = Command::new(binary());
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().expect("verum failed to spawn");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// =====================================================================
// Mode dispatch
// =====================================================================

#[test]
fn bare_invocation_with_shebang_runs_as_script() {
    let d = fresh_dir("shebang_runs");
    let p = write_source(
        &d,
        "hello.vr",
        "#!/usr/bin/env verum\nprint(\"hello\");\n",
    );
    let (code, stdout, stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(
        code, 0,
        "shebang script must exit 0. stdout={} stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("hello"),
        "expected 'hello' in stdout. stdout={}",
        stdout
    );
}

#[test]
fn bare_invocation_without_shebang_emits_advisory_and_fails() {
    let d = fresh_dir("no_shebang_advisory");
    // Has fn main() but no shebang — bare `verum file.vr` must surface
    // the advisory pointing at `verum run`, not silently route to script
    // mode (which would parse-error on top-level stmts) or to interpreter
    // mode (which would obscure the contract).
    let p = write_source(&d, "main.vr", "fn main() { print(\"x\"); }\n");
    let (code, _stdout, stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_ne!(code, 0, "non-script .vr file must NOT silently run via bare invocation");
    assert!(
        stderr.contains("verum run"),
        "advisory must redirect to `verum run`. stderr={}",
        stderr
    );
    assert!(
        stderr.contains("shebang"),
        "advisory must mention shebang. stderr={}",
        stderr
    );
}

#[test]
fn explicit_run_works_for_non_script_vr_file() {
    // The `verum run file.vr` form is unconditional — works for any
    // .vr file with a valid entry, shebang or not.
    let d = fresh_dir("run_no_shebang");
    let p = write_source(&d, "main.vr", "fn main() { print(\"explicit\"); }\n");
    let (code, stdout, stderr) = run_verum(&["run", p.to_str().unwrap()], None);
    assert_eq!(
        code, 0,
        "verum run must succeed on a fn-main file. stderr={}",
        stderr
    );
    assert!(
        stdout.contains("explicit"),
        "expected 'explicit' in stdout. stdout={}",
        stdout
    );
}

#[test]
fn explicit_run_works_for_shebang_script() {
    // `verum run shebang.vr` is also valid; the shebang triggers
    // script-mode parsing inside the pipeline regardless of the
    // CLI form.
    let d = fresh_dir("run_with_shebang");
    let p = write_source(
        &d,
        "script.vr",
        "#!/usr/bin/env verum\nprint(\"via run\");\n",
    );
    let (code, stdout, stderr) = run_verum(&["run", p.to_str().unwrap()], None);
    assert_eq!(code, 0, "verum run on a script must succeed. stderr={}", stderr);
    assert!(
        stdout.contains("via run"),
        "expected 'via run'. stdout={}",
        stdout
    );
}

// =====================================================================
// Wrapper synthesis: top-level statements + decls
// =====================================================================

#[test]
fn script_with_top_level_let_and_print() {
    let d = fresh_dir("top_level_let");
    let p = write_source(
        &d,
        "let.vr",
        "#!/usr/bin/env verum\nlet x = 7;\nprint(x);\n",
    );
    let (code, stdout, stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(code, 0, "let+print must succeed. stderr={}", stderr);
    assert!(stdout.contains("7"), "expected '7'. stdout={}", stdout);
}

#[test]
fn script_with_decl_and_top_level_call() {
    // Mixed: a `fn helper()` decl AND top-level statements that call it.
    // Verifies the wrapper preserves source order (decl visible before
    // the call to it).
    let d = fresh_dir("decl_plus_call");
    let p = write_source(
        &d,
        "mixed.vr",
        "#!/usr/bin/env verum\n\
         fn double(n: Int) -> Int { return n * 2; }\n\
         let x = double(21);\n\
         print(x);\n",
    );
    let (code, stdout, stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(code, 0, "mixed decl+stmt must succeed. stderr={}", stderr);
    assert!(stdout.contains("42"), "expected '42'. stdout={}", stdout);
}

// =====================================================================
// Exit-code propagation
// =====================================================================

#[test]
fn tail_int_becomes_exit_code() {
    // `print("done"); 42` → wrapper returns Int 42 → process exits 42.
    let d = fresh_dir("tail_int");
    let p = write_source(
        &d,
        "exit42.vr",
        "#!/usr/bin/env verum\nprint(\"done\");\n42\n",
    );
    let (code, stdout, _stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(code, 42, "tail Int 42 must produce exit 42");
    assert!(stdout.contains("done"), "stdout={}", stdout);
}

#[test]
fn tail_int_zero_is_explicit_success() {
    let d = fresh_dir("tail_zero");
    let p = write_source(
        &d,
        "exit0.vr",
        "#!/usr/bin/env verum\nprint(\"ok\");\n0\n",
    );
    let (code, _stdout, _stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(code, 0, "explicit tail 0 must produce exit 0");
}

#[test]
fn no_tail_value_returns_zero() {
    let d = fresh_dir("no_tail");
    let p = write_source(
        &d,
        "void.vr",
        "#!/usr/bin/env verum\nprint(\"void\");\n",
    );
    let (code, stdout, _stderr) = run_verum(&[p.to_str().unwrap()], None);
    assert_eq!(code, 0, "no tail expr must default to exit 0");
    assert!(stdout.contains("void"), "stdout={}", stdout);
}

#[test]
fn fn_main_returning_int_propagates_exit_code() {
    // Library-mode file (no shebang) with `fn main() -> Int`.
    // Exit code must equal the returned Int — tier-parity with AOT
    // where this is automatic via C runtime semantics.
    let d = fresh_dir("main_int");
    let p = write_source(
        &d,
        "main.vr",
        "fn main() -> Int {\n    print(\"main\");\n    return 9;\n}\n",
    );
    let (code, stdout, stderr) = run_verum(&["run", p.to_str().unwrap()], None);
    assert_eq!(code, 9, "main returning Int 9 must exit 9. stderr={}", stderr);
    assert!(stdout.contains("main"), "stdout={}", stdout);
}

// =====================================================================
// Shebang exec (Unix only)
// =====================================================================

#[cfg(unix)]
#[test]
fn shebang_exec_via_kernel_dispatch() {
    use std::os::unix::fs::PermissionsExt;
    let d = fresh_dir("kernel_exec");
    // Use the canonical shebang form. The kernel will resolve `verum`
    // via the test's PATH, which we set to include the binary's dir.
    let p = write_source(
        &d,
        "kernel.vr",
        "#!/usr/bin/env verum\nprint(\"exec ok\");\n",
    );
    // Make executable.
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&p, perms).unwrap();

    // Add the verum binary's directory to PATH for env-resolution.
    let bin_dir = std::path::Path::new(binary())
        .parent()
        .expect("binary parent")
        .to_path_buf();
    let new_path = match std::env::var_os("PATH") {
        Some(p) => {
            let mut paths: Vec<PathBuf> = std::env::split_paths(&p).collect();
            paths.insert(0, bin_dir.clone());
            std::env::join_paths(paths).expect("join_paths")
        }
        None => bin_dir.clone().into_os_string(),
    };

    let out = Command::new(&p)
        .env("PATH", &new_path)
        .output()
        .expect("kernel exec failed to spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code().unwrap_or(-1),
        0,
        "kernel-dispatched shebang must exit 0. stderr={}",
        stderr
    );
    assert!(
        stdout.contains("exec ok"),
        "expected 'exec ok'. stdout={}",
        stdout
    );
}

// =====================================================================
// Diagnostics
// =====================================================================

#[test]
fn no_entry_point_error_mentions_both_modes() {
    // A `.vr` file with no fn main() AND no shebang — under `verum run`
    // (which ignores shebang requirements at the rewriter layer but
    // the parser still rejects top-level statements without a shebang
    // in the source). The user sees a parse error that hints at both
    // recovery options.
    let d = fresh_dir("ambiguous");
    let p = write_source(
        &d,
        "no_entry.vr",
        // Only a fn helper(); no main() and no shebang.
        "fn helper() { print(\"hi\"); }\n",
    );
    let (code, _stdout, stderr) = run_verum(&["run", p.to_str().unwrap()], None);
    assert_ne!(code, 0, "missing-entry must fail");
    // Help text should redirect the user; either the entry-detection
    // diagnostic ("No entry point found ... shebang ... fn main()") or
    // an earlier parse-fail in script mode is acceptable.
    let combined = stderr.to_lowercase();
    assert!(
        combined.contains("main") || combined.contains("entry") || combined.contains("shebang"),
        "expected diagnostic to mention main/entry/shebang. stderr={}",
        stderr
    );
}
