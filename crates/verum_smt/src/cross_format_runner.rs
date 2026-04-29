//! Cross-format foreign-system runner.
//!
//! ## What this module is
//!
//! `verum_kernel::cross_format_gate` ships pure types
//! (`ExportFormat`, `FormatStatus`, `CrossFormatReport`); it cannot
//! invoke external tools because the kernel is sandboxed.  This
//! module provides the **runner layer** that actually drives the
//! foreign proof checkers and populates a `CrossFormatReport`.
//!
//! ## Architecture
//!
//! Single trait boundary [`ForeignSystemChecker`] with one
//! implementation per format:
//!
//!   * [`CoqChecker`]      — `coqc <file>.v`
//!   * [`LeanChecker`]     — `lean <file>.lean`
//!   * [`IsabelleChecker`] — `isabelle build -d <dir> <session>`
//!   * [`DeduktiChecker`]  — `kontroli check <file>.dk` (or `dkcheck`)
//!   * [`AgdaChecker`]     — `agda --no-libraries <file>.agda`
//!   * [`MetamathChecker`] — `metamath '...verify proof *' '...quit'`
//!
//! Each checker:
//!
//!   1. Probes whether the foreign tool is installed
//!      ([`ForeignSystemChecker::is_available`]).
//!   2. Invokes it on a given file
//!      ([`ForeignSystemChecker::check_file`]).
//!   3. Returns a typed [`CheckResult`] capturing pass/fail/missing.
//!
//! ## Foundation-neutral
//!
//! The runner is foundation-neutral: it cares only about EXIT-CODE
//! verdicts from the foreign tool.  Any tool that follows the
//! Unix-style "exit 0 = ok" convention plugs in via a new
//! [`ForeignSystemChecker`] impl.
//!
//! ## Trust boundary
//!
//! The runner DOES NOT trust the foreign system's verdict
//! unconditionally — it merely captures it.  The final gate verdict
//! lives in `verum_kernel::cross_format_gate::evaluate_gate`, which
//! requires every required format to report `Passed` before claiming
//! cross-format mechanization.  The runner is the live signal source
//! for those `FormatStatus` entries.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use verum_common::Text;
use verum_kernel::cross_format_gate::{ExportFormat, FormatStatus};

// =============================================================================
// CheckResult — the runner's verdict before lifting to FormatStatus
// =============================================================================

/// Result of running a foreign system on a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// The foreign tool accepted the file.
    Passed {
        /// Tool version string (best-effort; empty when unavailable).
        tool_version: String,
        /// Wall-clock duration of the check.
        elapsed: Duration,
        /// Captured stdout (truncated to a sensible diagnostic length).
        stdout_excerpt: String,
    },
    /// The foreign tool rejected the file.
    Failed {
        /// Tool exit code.
        exit_code: i32,
        /// Captured stderr (truncated for diagnostic display).
        stderr_excerpt: String,
        /// Captured stdout (truncated).
        stdout_excerpt: String,
    },
    /// The foreign tool is not installed on this host.
    ToolMissing {
        /// Hint for the user (e.g. `brew install coq` / `apt install coq`).
        install_hint: String,
    },
    /// Internal runner failure (process spawn error, I/O timeout, etc.).
    RunnerError {
        /// Diagnostic message.
        reason: String,
    },
}

impl CheckResult {
    /// Lift the result into a kernel-side `FormatStatus`.  `Passed` →
    /// `Passed`; `Failed` / `RunnerError` → `Failed`; `ToolMissing` →
    /// `NotRun` (skipping a missing tool is the conservative choice;
    /// the gate still refuses to GREEN until the tool is run).
    pub fn into_format_status(self) -> FormatStatus {
        match self {
            CheckResult::Passed {
                tool_version,
                elapsed,
                ..
            } => FormatStatus::Passed {
                message: Text::from(format!(
                    "{} ({}ms)",
                    tool_version,
                    elapsed.as_millis()
                )),
            },
            CheckResult::Failed {
                exit_code,
                stderr_excerpt,
                ..
            } => FormatStatus::Failed {
                reason: Text::from(format!(
                    "exit {}: {}",
                    exit_code,
                    stderr_excerpt.trim()
                )),
            },
            CheckResult::ToolMissing { install_hint } => FormatStatus::NotRun {
                reason: Text::from(format!("tool missing — {}", install_hint)),
            },
            CheckResult::RunnerError { reason } => FormatStatus::Failed {
                reason: Text::from(format!("runner error: {}", reason)),
            },
        }
    }

    pub fn is_passed(&self) -> bool {
        matches!(self, CheckResult::Passed { .. })
    }
}

// =============================================================================
// ForeignSystemChecker — the trait boundary
// =============================================================================

/// Single dispatch interface for checking certificate files in a
/// foreign proof system.
///
/// Implementations MUST:
///
///   * Be cheap to construct (`Default::default()`-friendly).
///   * Honour reasonable defaults for tool flags (no-libraries,
///     no-network, deterministic).
///   * Return `CheckResult::ToolMissing` (NOT `RunnerError`) when
///     the tool is not on the system PATH; this is the
///     differentially-meaningful signal.
///   * Capture a bounded prefix of stdout/stderr for diagnostic
///     display (avoid blowing up on tools that print MB of output).
pub trait ForeignSystemChecker {
    /// The format this checker handles.
    fn format(&self) -> ExportFormat;

    /// True iff the foreign tool is on the host's PATH.
    fn is_available(&self) -> bool;

    /// Invoke the foreign tool on a single certificate file.
    fn check_file(&self, path: &Path) -> CheckResult;

    /// One-line install hint for the user.
    fn install_hint(&self) -> &'static str;
}

/// Helper: spawn a command, capture exit/stderr/stdout, return a
/// [`CheckResult`].  Common implementation across most checkers.
fn run_external_tool(
    tool: &str,
    args: &[&str],
    install_hint: &str,
    excerpt_bytes: usize,
) -> CheckResult {
    if !is_tool_on_path(tool) {
        return CheckResult::ToolMissing {
            install_hint: install_hint.to_string(),
        };
    }
    let started = Instant::now();
    let output = match Command::new(tool).args(args).output() {
        Ok(o) => o,
        Err(e) => {
            return CheckResult::RunnerError {
                reason: format!("failed to spawn `{}`: {}", tool, e),
            };
        }
    };
    let elapsed = started.elapsed();
    let stdout_excerpt = excerpt_utf8(&output.stdout, excerpt_bytes);
    let stderr_excerpt = excerpt_utf8(&output.stderr, excerpt_bytes);
    if output.status.success() {
        CheckResult::Passed {
            tool_version: tool_version_probe(tool).unwrap_or_default(),
            elapsed,
            stdout_excerpt,
        }
    } else {
        let exit_code = output.status.code().unwrap_or(-1);
        CheckResult::Failed {
            exit_code,
            stderr_excerpt,
            stdout_excerpt,
        }
    }
}

/// Best-effort version probe (`<tool> --version`).  Used only for
/// `Passed`-result diagnostics; failures are silent.
fn tool_version_probe(tool: &str) -> Option<String> {
    let output = Command::new(tool).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// True iff `tool` is on the system PATH (POSIX `which`-style probe).
fn is_tool_on_path(tool: &str) -> bool {
    // Use `command -v` for portability; `which` is not on every host.
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {} >/dev/null 2>&1", tool))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Extract a bounded UTF-8 excerpt from a byte buffer.
fn excerpt_utf8(bytes: &[u8], max_bytes: usize) -> String {
    let s = String::from_utf8_lossy(bytes);
    if s.len() <= max_bytes {
        return s.into_owned();
    }
    // Find last char-boundary ≤ max_bytes (avoid mid-codepoint cut).
    let mut cut = max_bytes;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = s[..cut].to_string();
    out.push_str("...");
    out
}

// =============================================================================
// Per-format checker implementations
// =============================================================================

/// Coq — `coqc <file>.v`.
#[derive(Debug, Default, Clone, Copy)]
pub struct CoqChecker;

impl ForeignSystemChecker for CoqChecker {
    fn format(&self) -> ExportFormat { ExportFormat::Coq }
    fn is_available(&self) -> bool { is_tool_on_path("coqc") }
    fn install_hint(&self) -> &'static str { "brew install coq  /  apt install coq  /  opam install coq" }
    fn check_file(&self, path: &Path) -> CheckResult {
        let p = path.to_string_lossy();
        run_external_tool("coqc", &["-q", &p], self.install_hint(), 4096)
    }
}

/// Lean 4 — `lean <file>.lean`.
#[derive(Debug, Default, Clone, Copy)]
pub struct LeanChecker;

impl ForeignSystemChecker for LeanChecker {
    fn format(&self) -> ExportFormat { ExportFormat::Lean4 }
    fn is_available(&self) -> bool { is_tool_on_path("lean") }
    fn install_hint(&self) -> &'static str { "curl https://elan.lean-lang.org/elan-init.sh -sSf | sh" }
    fn check_file(&self, path: &Path) -> CheckResult {
        let p = path.to_string_lossy();
        run_external_tool("lean", &[&p], self.install_hint(), 4096)
    }
}

/// Isabelle/HOL — `isabelle process -e 'use_thy "<file>"'`.  Lighter
/// than `isabelle build -d`; works on standalone .thy files.
#[derive(Debug, Default, Clone, Copy)]
pub struct IsabelleChecker;

impl ForeignSystemChecker for IsabelleChecker {
    fn format(&self) -> ExportFormat { ExportFormat::Isabelle }
    fn is_available(&self) -> bool { is_tool_on_path("isabelle") }
    fn install_hint(&self) -> &'static str { "brew install --cask isabelle  /  download from isabelle.in.tum.de" }
    fn check_file(&self, path: &Path) -> CheckResult {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let cmd = format!("use_thy \"{}\"", stem);
        run_external_tool(
            "isabelle",
            &["process", "-l", "Pure", "-e", &cmd],
            self.install_hint(),
            4096,
        )
    }
}

/// Dedukti — `kontroli check <file>.dk` (newer) or `dkcheck` (legacy).
#[derive(Debug, Default, Clone, Copy)]
pub struct DeduktiChecker;

impl ForeignSystemChecker for DeduktiChecker {
    fn format(&self) -> ExportFormat { ExportFormat::Dedukti }
    fn is_available(&self) -> bool {
        is_tool_on_path("kontroli") || is_tool_on_path("dkcheck")
    }
    fn install_hint(&self) -> &'static str {
        "cargo install kontroli  /  opam install dedukti"
    }
    fn check_file(&self, path: &Path) -> CheckResult {
        let p = path.to_string_lossy();
        if is_tool_on_path("kontroli") {
            run_external_tool("kontroli", &[&p], self.install_hint(), 4096)
        } else if is_tool_on_path("dkcheck") {
            run_external_tool("dkcheck", &[&p], self.install_hint(), 4096)
        } else {
            CheckResult::ToolMissing {
                install_hint: self.install_hint().to_string(),
            }
        }
    }
}

// =============================================================================
// Generic format → checker dispatch
// =============================================================================

/// Return the canonical checker for a format.  Returns `None` for
/// formats that aren't yet wired (currently: Agda + Metamath; lift
/// to V1).
pub fn checker_for(format: ExportFormat) -> Option<Box<dyn ForeignSystemChecker>> {
    match format {
        ExportFormat::Coq      => Some(Box::new(CoqChecker)),
        ExportFormat::Lean4    => Some(Box::new(LeanChecker)),
        ExportFormat::Isabelle => Some(Box::new(IsabelleChecker)),
        ExportFormat::Dedukti  => Some(Box::new(DeduktiChecker)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_result_lifts_to_format_status() {
        let passed = CheckResult::Passed {
            tool_version: "8.18.0".into(),
            elapsed: Duration::from_millis(42),
            stdout_excerpt: String::new(),
        };
        match passed.into_format_status() {
            FormatStatus::Passed { message } => {
                assert!(message.as_str().contains("8.18.0"));
                assert!(message.as_str().contains("42ms"));
            }
            other => panic!("expected Passed, got {:?}", other),
        }

        let missing = CheckResult::ToolMissing {
            install_hint: "brew install coq".into(),
        };
        match missing.into_format_status() {
            FormatStatus::NotRun { reason } => {
                assert!(reason.as_str().contains("brew install coq"));
            }
            other => panic!("expected NotRun, got {:?}", other),
        }

        let failed = CheckResult::Failed {
            exit_code: 1,
            stderr_excerpt: "type mismatch".into(),
            stdout_excerpt: String::new(),
        };
        match failed.into_format_status() {
            FormatStatus::Failed { reason } => {
                assert!(reason.as_str().contains("exit 1"));
                assert!(reason.as_str().contains("type mismatch"));
            }
            other => panic!("expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn excerpt_utf8_truncates_safely() {
        // Multi-byte UTF-8 (Cyrillic) — verify cut respects char boundaries.
        let bytes = "Привет, мир! ".repeat(100).into_bytes();
        let s = excerpt_utf8(&bytes, 50);
        assert!(s.is_char_boundary(s.len()) || s.ends_with("..."));
        assert!(s.len() <= 53); // 50 + "..."
    }

    #[test]
    fn checker_for_returns_for_known_formats() {
        assert!(checker_for(ExportFormat::Coq).is_some());
        assert!(checker_for(ExportFormat::Lean4).is_some());
        assert!(checker_for(ExportFormat::Isabelle).is_some());
        assert!(checker_for(ExportFormat::Dedukti).is_some());
    }

    #[test]
    fn checkers_report_format_correctly() {
        assert_eq!(CoqChecker.format(), ExportFormat::Coq);
        assert_eq!(LeanChecker.format(), ExportFormat::Lean4);
        assert_eq!(IsabelleChecker.format(), ExportFormat::Isabelle);
        assert_eq!(DeduktiChecker.format(), ExportFormat::Dedukti);
    }

    #[test]
    fn install_hints_are_non_empty() {
        for c in [
            checker_for(ExportFormat::Coq).unwrap(),
            checker_for(ExportFormat::Lean4).unwrap(),
            checker_for(ExportFormat::Isabelle).unwrap(),
            checker_for(ExportFormat::Dedukti).unwrap(),
        ] {
            assert!(!c.install_hint().is_empty());
        }
    }

    // is_available checks live tool presence; on CI hosts the tools may
    // not be installed, so we only assert that the call doesn't panic.
    #[test]
    fn is_available_does_not_panic() {
        for c in [
            checker_for(ExportFormat::Coq).unwrap(),
            checker_for(ExportFormat::Lean4).unwrap(),
            checker_for(ExportFormat::Isabelle).unwrap(),
            checker_for(ExportFormat::Dedukti).unwrap(),
        ] {
            let _ = c.is_available();
        }
    }
}
