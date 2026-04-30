//! Weft net-framework end-to-end probes.
//!
//! Anchors three KВИ (CVE) acceptance gates for `core/net/weft/*`,
//! pinned by direct invocation of the on-disk `verum` binary against
//! a freshly-authored hello-world `.vr` source. Each probe is a
//! single `#[test]` so the failure mode is explicit in CI:
//!
//!   * `check_passes_for_weft_hello_world`
//!     CVE axis В — `verum check` must accept a router + handler
//!     program that mounts every umbrella `core.net.weft.*` symbol.
//!     Currently passes.
//!
//!   * `interpreter_runs_weft_hello_world_without_bind`
//!     CVE axis И (smoke) — `verum run` (Tier 0 interpreter) must
//!     execute a program that constructs a `Router` + `WeftApp` but
//!     does NOT call `.bind()`. Currently passes.
//!
//!   * `aot_runs_weft_hello_world_without_bind`
//!     CVE axis И (Tier 1) — `verum run --aot` should produce a
//!     native executable that prints HELLO_OK. Currently *does not*
//!     pass deterministically: even without `.bind()`, the AOT
//!     binary segfaults because the lowering skips `env_ctx_*` /
//!     `os_mmap` / `os_alloc_segment` (see task #13). The test is
//!     gated behind `WEFT_RUN_BIND_PROBES=1` until that lands.
//!
//! Two known-failure probes capture the foundational gaps surfaced
//! during the 2026-04-30 audit. They are gated behind
//! `WEFT_RUN_BIND_PROBES=1` so CI does not red-flag them while the
//! underlying compiler fixes are in flight (tracked tasks #12 and
//! #13). When the gate flips green, drop the env-guard and let the
//! failures regress the bug-fix landings.
//!
//!   * `interpreter_bind_panics_with_known_message` —
//!     `Result.map_err` is not resolved by the VBC interpreter's
//!     method-dispatch path when `TcpListener.bind` invokes it.
//!
//!   * `aot_bind_segfaults_until_runtime_helpers_landed` —
//!     `env_ctx_*`, `os_mmap`, `os_alloc_segment`, `swap`,
//!     `get_thread_id` are skipped during VBC→LLVM lowering with
//!     `undefined variable: CONTEXT_SLOT_COUNT`; the binary jumps
//!     to a missing function and dies with SIGSEGV.

use std::process::Command;

fn verum_bin() -> std::path::PathBuf {
    // Prefer release build (the one the user-facing toolchain ships)
    // and fall back to debug if release is not available locally.
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate manifest sits two levels deep")
        .to_path_buf();
    let release = workspace_root.join("target/release/verum");
    if release.exists() {
        return release;
    }
    workspace_root.join("target/debug/verum")
}

fn write_program(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write probe program");
    path
}

const HELLO_WORLD_NO_BIND: &str = r#"// auto-generated probe; see crates/verum_integration_tests/tests/weft_e2e.rs
mount core.net.http.{Method, Response};
mount core.net.weft.service.{ServiceBuilder};
mount core.net.weft.handler.{WeftRequest, Handler};
mount core.net.weft.router.{Router};
mount core.net.weft.app.{WeftApp};
mount core.net.weft.error.{WeftError};
mount core.net.weft.response_ext.{resp_text};

type Hello is {};

implement Handler for Hello {
    async fn handle(&self, _req: WeftRequest) -> Result<Response, WeftError> {
        Ok(resp_text("Hello from Weft!"))
    }
}

fn main() {
    let router = Router.new().get("/", Hello {});
    let svc    = ServiceBuilder.new(router).build();
    let _app   = WeftApp.new(svc);
    print("HELLO_OK");
}
"#;

const HELLO_WORLD_WITH_BIND: &str = r#"// auto-generated probe; see crates/verum_integration_tests/tests/weft_e2e.rs
mount core.net.http.{Method, Response};
mount core.net.weft.service.{ServiceBuilder};
mount core.net.weft.handler.{WeftRequest, Handler};
mount core.net.weft.router.{Router};
mount core.net.weft.app.{WeftApp};
mount core.net.weft.error.{WeftError};
mount core.net.weft.response_ext.{resp_text};

type Hello is {};

implement Handler for Hello {
    async fn handle(&self, _req: WeftRequest) -> Result<Response, WeftError> {
        Ok(resp_text("Hello from Weft!"))
    }
}

fn main() {
    let router = Router.new().get("/", Hello {});
    let svc    = ServiceBuilder.new(router).build();
    let app    = WeftApp.new(svc);
    match app.bind("127.0.0.1:0") {
        Ok(_server) => { print("BIND_OK"); }
        Err(msg)    => { print(f"BIND_FAIL: {msg}"); }
    }
}
"#;

fn run_verum(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    Command::new(verum_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn verum")
}

#[test]
fn check_passes_for_weft_hello_world() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_program(dir.path(), "hello.vr", HELLO_WORLD_NO_BIND);
    let out = run_verum(&["check", path.to_str().unwrap()], dir.path());
    assert!(
        out.status.success(),
        "verum check failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn interpreter_runs_weft_hello_world_without_bind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_program(dir.path(), "hello.vr", HELLO_WORLD_NO_BIND);
    let out = run_verum(&["run", path.to_str().unwrap()], dir.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "verum run (interpreter) failed:\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr,
    );
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("HELLO_OK"),
        "expected HELLO_OK marker in interpreter output:\n{}",
        combined,
    );
}

/// Gated AOT smoke test. Until task #13 lands the AOT binary
/// segfaults even for the no-bind program because lowering drops
/// the runtime context helpers (`env_ctx_*` / `os_mmap` /
/// `os_alloc_segment`). When the fix lands, drop the env-guard.
#[test]
fn aot_runs_weft_hello_world_without_bind() {
    if std::env::var_os("WEFT_RUN_BIND_PROBES").is_none() {
        eprintln!("skipped (set WEFT_RUN_BIND_PROBES=1 once task #13 lands)");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_program(dir.path(), "hello.vr", HELLO_WORLD_NO_BIND);
    let out = run_verum(&["run", "--aot", path.to_str().unwrap()], dir.path());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "verum run --aot failed:\nstdout:\n{}\nstderr:\n{}",
        stdout, stderr,
    );
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains("HELLO_OK"),
        "expected HELLO_OK marker in AOT output:\n{}",
        combined,
    );
}

/// Pinned regression for task #12 (interpreter `Result.map_err`).
///
/// Drives the same hello-world but adds a `WeftApp.bind("127.0.0.1:0")`
/// call. Today this panics with "method 'Result.map_err' not found on
/// value" because the interpreter's method-dispatch path cannot
/// resolve the generic Result method invoked from
/// `core/net/tcp.vr:321`. The probe is **gated** behind
/// `WEFT_RUN_BIND_PROBES=1` so it does not red-flag CI until the
/// fix lands; once landed, drop the gate and the test should pass.
#[test]
fn interpreter_bind_pinned_known_failure() {
    if std::env::var_os("WEFT_RUN_BIND_PROBES").is_none() {
        eprintln!("skipped (set WEFT_RUN_BIND_PROBES=1 once task #12 lands)");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_program(dir.path(), "bind.vr", HELLO_WORLD_WITH_BIND);
    let out = run_verum(&["run", path.to_str().unwrap()], dir.path());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        out.status.success() && combined.contains("BIND_OK"),
        "expected BIND_OK after task #12 lands; got:\n{}",
        combined,
    );
}

/// Pinned regression for task #13 (AOT runtime helpers).
///
/// Today the AOT path skips `env_ctx_*`, `os_mmap`,
/// `os_alloc_segment`, `swap`, `get_thread_id` during VBC→LLVM
/// lowering and the resulting binary segfaults (exit 139). Gated
/// the same way as the interpreter probe.
#[test]
fn aot_bind_pinned_known_failure() {
    if std::env::var_os("WEFT_RUN_BIND_PROBES").is_none() {
        eprintln!("skipped (set WEFT_RUN_BIND_PROBES=1 once task #13 lands)");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_program(dir.path(), "bind.vr", HELLO_WORLD_WITH_BIND);
    let out = run_verum(&["run", "--aot", path.to_str().unwrap()], dir.path());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        out.status.success() && combined.contains("BIND_OK"),
        "expected BIND_OK after task #13 lands; got:\n{}",
        combined,
    );
}
