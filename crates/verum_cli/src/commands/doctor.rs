//! `verum doctor` — surveys the user's Verum installation and reports
//! actionable health-check results.
//!
//! Each check returns a [`CheckResult`] with a status (`Pass` /
//! `Warn` / `Fail`), a one-line summary, and an optional hint
//! pointing at the documented remediation. Output is human-readable
//! by default; `--json` emits one NDJSON object per check. Exit
//! codes:
//!
//!   - `0` — every check passed
//!   - `1` — at least one warning (only when `--strict`)
//!   - `2` — at least one failure
//!
//! Without `--strict`, warnings exit 0 — useful for CI gates that
//! only want to fail on hard breakage.

use crate::error::{CliError, Result};
use crate::script::cache::ScriptCache;
use crate::script::permissions::{Permission, PermissionKind};
use crate::registry::content_store::ContentStore;
use crate::registry::lockfile_v3::{LockfileV3, SCHEMA_VERSION};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(clap::Args, Debug, Clone, Default)]
pub struct DoctorArgs {
    /// Emit one NDJSON object per check instead of human output.
    #[clap(long)]
    pub json: bool,
    /// Exit non-zero on warnings as well as failures.
    #[clap(long)]
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }

    fn json_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    /// Short identifier (lowercase, snake_case) — stable for scripts.
    pub id: &'static str,
    pub status: Status,
    /// One-line user-facing summary (no trailing newline).
    pub summary: String,
    /// Optional remediation hint.
    pub hint: Option<String>,
}

pub fn execute(args: DoctorArgs) -> Result<()> {
    let results = run_all_checks();
    let max_status = results
        .iter()
        .map(|r| r.status)
        .fold(Status::Pass, max_status);

    if args.json {
        emit_json(&results)?;
    } else {
        emit_human(&results, max_status)?;
    }

    let exit = match (max_status, args.strict) {
        (Status::Fail, _) => 2,
        (Status::Warn, true) => 1,
        _ => 0,
    };
    if exit != 0 {
        std::process::exit(exit);
    }
    Ok(())
}

fn max_status(a: Status, b: Status) -> Status {
    match (a, b) {
        (Status::Fail, _) | (_, Status::Fail) => Status::Fail,
        (Status::Warn, _) | (_, Status::Warn) => Status::Warn,
        _ => Status::Pass,
    }
}

fn emit_human(results: &[CheckResult], summary_status: Status) -> Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "verum doctor — toolchain v{}", env!("CARGO_PKG_VERSION"))
        .map_err(CliError::Io)?;
    writeln!(out).map_err(CliError::Io)?;
    let mut name_w = 0usize;
    for r in results {
        if r.id.len() > name_w {
            name_w = r.id.len();
        }
    }
    for r in results {
        writeln!(
            out,
            "  [{}] {:<width$}  {}",
            r.status.label(),
            r.id,
            r.summary,
            width = name_w
        )
        .map_err(CliError::Io)?;
        if let Some(h) = &r.hint {
            writeln!(out, "         {:<width$}  hint: {}", "", h, width = name_w)
                .map_err(CliError::Io)?;
        }
    }
    writeln!(out).map_err(CliError::Io)?;
    let pass = results.iter().filter(|r| r.status == Status::Pass).count();
    let warn = results.iter().filter(|r| r.status == Status::Warn).count();
    let fail = results.iter().filter(|r| r.status == Status::Fail).count();
    writeln!(
        out,
        "summary: {} pass, {} warn, {} fail (overall {})",
        pass,
        warn,
        fail,
        summary_status.label()
    )
    .map_err(CliError::Io)?;
    Ok(())
}

fn emit_json(results: &[CheckResult]) -> Result<()> {
    let mut out = io::stdout().lock();
    for r in results {
        let hint = r
            .hint
            .as_ref()
            .map(|h| format!(",\"hint\":{}", json_str(h)))
            .unwrap_or_default();
        writeln!(
            out,
            r#"{{"id":"{}","status":"{}","summary":{}{}}}"#,
            r.id,
            r.status.json_str(),
            json_str(&r.summary),
            hint,
        )
        .map_err(CliError::Io)?;
    }
    Ok(())
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn run_all_checks() -> Vec<CheckResult> {
    let mut results = Vec::with_capacity(8);
    results.push(check_toolchain_version());
    let home = check_home_directory(&mut results);
    if let Some(home) = &home {
        results.push(check_verum_directory(home));
        results.push(check_script_cache(home));
        results.push(check_content_store(home));
    }
    results.push(check_permission_grammar());
    results.push(check_project_lockfile_if_present(std::env::current_dir().ok().as_deref()));
    results
}

// ── individual checks ────────────────────────────────────────────────

fn check_toolchain_version() -> CheckResult {
    CheckResult {
        id: "toolchain_version",
        status: Status::Pass,
        summary: format!(
            "verum CLI v{} (built against rustc target {})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
        ),
        hint: None,
    }
}

fn check_home_directory(out: &mut Vec<CheckResult>) -> Option<PathBuf> {
    match dirs::home_dir() {
        Some(h) => {
            out.push(CheckResult {
                id: "home_directory",
                status: Status::Pass,
                summary: format!("$HOME = {}", h.display()),
                hint: None,
            });
            Some(h)
        }
        None => {
            out.push(CheckResult {
                id: "home_directory",
                status: Status::Fail,
                summary: "$HOME could not be resolved".into(),
                hint: Some("set $HOME (Unix) or %USERPROFILE% (Windows) and retry".into()),
            });
            None
        }
    }
}

fn check_verum_directory(home: &Path) -> CheckResult {
    let dir = home.join(".verum");
    match std::fs::create_dir_all(&dir) {
        Ok(()) => {
            // Probe writability with a temp file.
            let probe = dir.join(".doctor-probe");
            match std::fs::write(&probe, b"ok") {
                Ok(()) => {
                    let _ = std::fs::remove_file(&probe);
                    CheckResult {
                        id: "verum_directory",
                        status: Status::Pass,
                        summary: format!("{} writable", dir.display()),
                        hint: None,
                    }
                }
                Err(e) => CheckResult {
                    id: "verum_directory",
                    status: Status::Fail,
                    summary: format!("{} not writable: {e}", dir.display()),
                    hint: Some("check disk space and directory permissions".into()),
                },
            }
        }
        Err(e) => CheckResult {
            id: "verum_directory",
            status: Status::Fail,
            summary: format!("cannot create {}: {e}", dir.display()),
            hint: Some("check parent directory permissions".into()),
        },
    }
}

fn check_script_cache(home: &Path) -> CheckResult {
    let root = home.join(".verum").join("script-cache");
    match ScriptCache::at(root.clone()) {
        Ok(cache) => match cache.list() {
            Ok(entries) => {
                let total: u64 = entries.iter().map(|(_, m)| m.vbc_len).sum();
                CheckResult {
                    id: "script_cache",
                    status: Status::Pass,
                    summary: format!(
                        "{} entries, {} ({})",
                        entries.len(),
                        format_bytes(total),
                        cache.root().display()
                    ),
                    hint: None,
                }
            }
            Err(e) => CheckResult {
                id: "script_cache",
                status: Status::Warn,
                summary: format!("cannot enumerate cache: {e}"),
                hint: Some("`verum cache clear` will rebuild from scratch".into()),
            },
        },
        Err(e) => CheckResult {
            id: "script_cache",
            status: Status::Fail,
            summary: format!("cannot open script cache: {e}"),
            hint: Some(format!("ensure {} is writable", root.display())),
        },
    }
}

fn check_content_store(home: &Path) -> CheckResult {
    let root = home.join(".verum").join("store");
    match ContentStore::at(root.clone()) {
        Ok(store) => match store.list() {
            Ok(entries) => {
                let total: u64 = entries.iter().map(|(_, m)| m.size).sum();
                let refs_count = match store.refs() {
                    Ok(r) => r.len(),
                    Err(_) => 0,
                };
                // Sample integrity check: pick first entry and verify.
                let mut integrity_note = String::new();
                if let Some((digest, _)) = entries.first() {
                    match store.lookup_by_digest(*digest) {
                        Ok(Some(_)) => {}
                        Ok(None) => integrity_note = " (sample lookup miss)".into(),
                        Err(e) => {
                            return CheckResult {
                                id: "content_store",
                                status: Status::Fail,
                                summary: format!(
                                    "integrity failure on sample entry: {e}"
                                ),
                                hint: Some(
                                    "the entry has been evicted; rerun \
                                     `verum install` to repopulate"
                                        .into(),
                                ),
                            };
                        }
                    }
                }
                CheckResult {
                    id: "content_store",
                    status: Status::Pass,
                    summary: format!(
                        "{} blobs / {} refs, {}{} ({})",
                        entries.len(),
                        refs_count,
                        format_bytes(total),
                        integrity_note,
                        store.root().display(),
                    ),
                    hint: None,
                }
            }
            Err(e) => CheckResult {
                id: "content_store",
                status: Status::Warn,
                summary: format!("cannot enumerate store: {e}"),
                hint: None,
            },
        },
        Err(e) => CheckResult {
            id: "content_store",
            status: Status::Fail,
            summary: format!("cannot open content store: {e}"),
            hint: Some(format!("ensure {} is writable", root.display())),
        },
    }
}

fn check_permission_grammar() -> CheckResult {
    // Compile-time guarantee: if the grammar parser is broken this
    // module won't compile. The check exists so the user can SEE the
    // grammar surface in `verum doctor` output.
    let kinds = [
        PermissionKind::FsRead,
        PermissionKind::FsWrite,
        PermissionKind::Net,
        PermissionKind::Env,
        PermissionKind::Run,
        PermissionKind::Ffi,
        PermissionKind::Time,
        PermissionKind::Random,
    ];
    // Sanity-parse a representative scope per kind to catch any
    // future regression that breaks the grammar wiring.
    for sample in [
        "fs:read",
        "fs:read=./data",
        "fs:write=./out,/tmp",
        "net",
        "net=api.example.com:443",
        "env=PATH",
        "run=git",
        "ffi=libc",
        "time",
        "random",
    ] {
        if Permission::parse(sample).is_err() {
            return CheckResult {
                id: "permission_grammar",
                status: Status::Fail,
                summary: format!("permission grammar regression on {sample:?}"),
                hint: Some("file an issue at github.com/verum-lang/verum".into()),
            };
        }
    }
    CheckResult {
        id: "permission_grammar",
        status: Status::Pass,
        summary: format!("{} kinds recognised, samples parse OK", kinds.len()),
        hint: None,
    }
}

fn check_project_lockfile_if_present(cwd: Option<&Path>) -> CheckResult {
    let cwd = match cwd {
        Some(c) => c,
        None => {
            return CheckResult {
                id: "project_lockfile",
                status: Status::Warn,
                summary: "could not determine working directory".into(),
                hint: None,
            };
        }
    };
    let lock = cwd.join("verum.lock");
    if !lock.is_file() {
        return CheckResult {
            id: "project_lockfile",
            status: Status::Pass,
            summary: "no verum.lock at cwd (not a project — skipped)".into(),
            hint: None,
        };
    }
    match LockfileV3::from_file(&lock) {
        Ok(lf) => CheckResult {
            id: "project_lockfile",
            status: Status::Pass,
            summary: format!(
                "verum.lock v{} — {} packages, root {:?}",
                lf.version,
                lf.packages.len(),
                lf.root
            ),
            hint: None,
        },
        Err(e) => {
            // V3 lockfile load failed. Could be a v1 lockfile, schema
            // skew, or self-integrity break — surface the detail.
            CheckResult {
                id: "project_lockfile",
                status: Status::Warn,
                summary: format!("verum.lock parse: {e}"),
                hint: Some(format!(
                    "expected v{SCHEMA_VERSION}; run `verum update` to regenerate"
                )),
            }
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────

fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if n >= GB {
        format!("{:.2} GiB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.2} MiB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.2} KiB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn max_status_promotes_to_strongest() {
        assert_eq!(max_status(Status::Pass, Status::Pass), Status::Pass);
        assert_eq!(max_status(Status::Pass, Status::Warn), Status::Warn);
        assert_eq!(max_status(Status::Warn, Status::Pass), Status::Warn);
        assert_eq!(max_status(Status::Warn, Status::Fail), Status::Fail);
        assert_eq!(max_status(Status::Pass, Status::Fail), Status::Fail);
        assert_eq!(max_status(Status::Fail, Status::Pass), Status::Fail);
    }

    #[test]
    fn status_label_and_json_str_are_stable() {
        assert_eq!(Status::Pass.label(), "PASS");
        assert_eq!(Status::Warn.label(), "WARN");
        assert_eq!(Status::Fail.label(), "FAIL");
        assert_eq!(Status::Pass.json_str(), "pass");
        assert_eq!(Status::Warn.json_str(), "warn");
        assert_eq!(Status::Fail.json_str(), "fail");
    }

    #[test]
    fn check_permission_grammar_passes() {
        let r = check_permission_grammar();
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains("8 kinds"));
    }

    #[test]
    fn check_toolchain_version_carries_pkg_version() {
        let r = check_toolchain_version();
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn check_verum_directory_passes_in_writable_root() {
        let tmp = TempDir::new().unwrap();
        let r = check_verum_directory(tmp.path());
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains(".verum"));
        // Probe file should have been cleaned up.
        assert!(!tmp.path().join(".verum").join(".doctor-probe").exists());
    }

    #[test]
    fn check_script_cache_passes_on_empty() {
        let tmp = TempDir::new().unwrap();
        let r = check_script_cache(tmp.path());
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains("0 entries"));
    }

    #[test]
    fn check_content_store_passes_on_empty() {
        let tmp = TempDir::new().unwrap();
        let r = check_content_store(tmp.path());
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains("0 blobs"));
    }

    #[test]
    fn check_project_lockfile_skipped_outside_project() {
        let tmp = TempDir::new().unwrap();
        let r = check_project_lockfile_if_present(Some(tmp.path()));
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains("not a project"));
    }

    #[test]
    fn check_project_lockfile_warns_on_garbage() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("verum.lock"), b":::not toml:::").unwrap();
        let r = check_project_lockfile_if_present(Some(tmp.path()));
        assert_eq!(r.status, Status::Warn);
        assert!(r.summary.contains("verum.lock"));
        assert!(r.hint.as_ref().unwrap().contains("regenerate"));
    }

    #[test]
    fn check_project_lockfile_passes_for_valid_v3() {
        let tmp = TempDir::new().unwrap();
        let mut lf = LockfileV3::new("test-app");
        lf.to_file(&tmp.path().join("verum.lock")).unwrap();
        let r = check_project_lockfile_if_present(Some(tmp.path()));
        assert_eq!(r.status, Status::Pass);
        assert!(r.summary.contains("test-app"));
    }

    #[test]
    fn json_str_escapes_specials() {
        assert_eq!(json_str("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str("x\\y"), "\"x\\\\y\"");
        assert_eq!(json_str("\n"), "\"\\n\"");
        assert_eq!(json_str("\x01"), "\"\\u0001\"");
    }

    #[test]
    fn format_bytes_uses_correct_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.00 KiB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GiB");
    }
}
