//! Solver-diagnostic side channel — the consumer-side of the
//! `VERUM_DUMP_SMT_DIR` / `VERUM_SOLVER_PROTOCOL` env-vars
//! exported by `verum verify --dump-smt` / `--solver-protocol`
//! (task #67).
//!
//! This module is the single shared surface that solver
//! backends (z3_backend, cvc5_backend, smtlib_check) call to
//! emit per-query dumps and per-command protocol traces.
//! Centralising the protocol in one module means:
//!
//!   1. The env-var names live in exactly one place — future
//!      renames touch one module.
//!   2. Every solver emits the same format, so an IDE that
//!      scrapes the dump dir / protocol log doesn't have to
//!      special-case each backend.
//!   3. The CLI's flag → env-var → consumer contract is
//!      testable end-to-end.
//!
//! The helpers are no-ops when the env vars are absent — calling
//! `dump_smt_query` in a release build without `--dump-smt` set
//! is pay-for-only-what-you-use.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Env var that, when set, points at a directory where each
/// solver query is written as `<prefix>-<counter>.smt2`.
pub const DUMP_DIR_ENV: &str = "VERUM_DUMP_SMT_DIR";

/// Env var that, when `"1"`, causes each `send` / `recv` to log
/// to stderr with a `[→]` / `[←]` prefix.
pub const PROTOCOL_ENV: &str = "VERUM_SOLVER_PROTOCOL";

/// Per-process counter for query-dump filenames.
static QUERY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// If `VERUM_DUMP_SMT_DIR` is set, return its value as a
/// `PathBuf`; otherwise `None`. The directory is assumed to
/// exist (the CLI creates it eagerly when `--dump-smt` is
/// parsed).
pub fn dump_smt_dir() -> Option<PathBuf> {
    std::env::var_os(DUMP_DIR_ENV).map(PathBuf::from)
}

/// Is the solver-protocol logger enabled?
pub fn protocol_enabled() -> bool {
    match std::env::var(PROTOCOL_ENV) {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    }
}

/// Dump an SMT-LIB query to the configured directory.
///
/// `prefix` names the caller (e.g. `"z3-obligation"` or
/// `"cvc5-subgoal"`); the filename is
/// `<prefix>-<counter>.smt2`. Silently no-ops when the env var
/// is unset — safe to call unconditionally.
///
/// Returns the path written to (for callers that want to log
/// the location), or None if dumping is disabled / the write
/// failed.
pub fn dump_smt_query(prefix: &str, content: &str) -> Option<PathBuf> {
    let dir = dump_smt_dir()?;
    let n = QUERY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{}-{:05}.smt2", prefix, n));
    match std::fs::write(&path, content) {
        Ok(()) => Some(path),
        Err(e) => {
            // Diagnostic failures don't fail verification;
            // surface on stderr so the user sees the skip
            // reason without crashing the build.
            eprintln!(
                "warning: --dump-smt: failed to write {}: {}",
                path.display(),
                e
            );
            None
        }
    }
}

/// Log a solver-protocol send line to stderr. No-op when the
/// env var is unset.
pub fn log_send(cmd: &str) {
    if protocol_enabled() {
        eprintln!("[→] {}", cmd.trim_end());
    }
}

/// Log a solver-protocol receive line to stderr. No-op when
/// the env var is unset.
pub fn log_recv(resp: &str) {
    if protocol_enabled() {
        eprintln!("[←] {}", resp.trim_end());
    }
}

/// Reset the query counter — used in tests so counter state
/// doesn't leak across test boundaries.
#[cfg(test)]
pub fn reset_counter_for_testing() {
    QUERY_COUNTER.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env-var mutation races against parallel test execution —
    // serialise every env-touching test under a single module
    // lock. The lock is fine-grained enough that the non-env
    // tests in this module don't contend.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_env_vars<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
        f();
        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(&k, &val) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }

    #[test]
    fn dump_smt_dir_returns_none_when_unset() {
        with_env_vars(&[(DUMP_DIR_ENV, None)], || {
            assert!(dump_smt_dir().is_none());
        });
    }

    #[test]
    fn dump_smt_dir_returns_path_when_set() {
        with_env_vars(&[(DUMP_DIR_ENV, Some("/tmp/verum-dump"))], || {
            let d = dump_smt_dir().expect("should be Some");
            assert_eq!(d.to_str(), Some("/tmp/verum-dump"));
        });
    }

    #[test]
    fn protocol_enabled_reads_env_var() {
        with_env_vars(&[(PROTOCOL_ENV, Some("1"))], || {
            assert!(protocol_enabled());
        });
        with_env_vars(&[(PROTOCOL_ENV, Some("true"))], || {
            assert!(protocol_enabled());
        });
        with_env_vars(&[(PROTOCOL_ENV, Some("TRUE"))], || {
            assert!(protocol_enabled());
        });
        with_env_vars(&[(PROTOCOL_ENV, Some("0"))], || {
            assert!(!protocol_enabled());
        });
        with_env_vars(&[(PROTOCOL_ENV, None)], || {
            assert!(!protocol_enabled());
        });
    }

    #[test]
    fn dump_smt_query_writes_to_configured_dir() {
        let tmpdir = std::env::temp_dir().join(format!(
            "verum-dump-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&tmpdir).unwrap();
        with_env_vars(
            &[(DUMP_DIR_ENV, Some(tmpdir.to_str().unwrap()))],
            || {
                reset_counter_for_testing();
                let path = dump_smt_query("test", "(check-sat)").unwrap();
                assert!(path.exists());
                let content = std::fs::read_to_string(&path).unwrap();
                assert_eq!(content, "(check-sat)");
                let fname = path.file_name().unwrap().to_string_lossy();
                assert!(fname.starts_with("test-"));
                assert!(fname.ends_with(".smt2"));
            },
        );
        std::fs::remove_dir_all(&tmpdir).ok();
    }

    #[test]
    fn dump_smt_query_is_noop_when_env_unset() {
        with_env_vars(&[(DUMP_DIR_ENV, None)], || {
            assert!(dump_smt_query("x", "y").is_none());
        });
    }

    #[test]
    fn log_send_and_log_recv_are_noop_when_disabled() {
        with_env_vars(&[(PROTOCOL_ENV, None)], || {
            // These should not panic and should not print
            // anything. We can't easily capture stderr in a
            // unit test without extra plumbing, but the
            // pay-for-only-what-you-use contract is that
            // they're cheap function calls when disabled.
            log_send("(assert x)");
            log_recv("sat");
        });
    }
}
