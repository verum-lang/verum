//! Multi-shell × multi-tier differential testing.
//!
//! Complements the in-tree `executor.rs::execute_differential` (which compares
//! interpreter vs AOT for the same Verum source) with a higher-dimensional
//! variant that fans out across:
//!
//!   * shell flavours — bash / zsh / dash / fish / pwsh
//!   * execution tiers — interpreter (Tier 0) / JIT (Tier 1) / AOT (Tier 2)
//!
//! Failure mode: any pair `(backend_a, tier_a)` vs `(backend_b, tier_b)` that
//! produces *structurally different* output (after normalisation) marks the
//! test as failing with a structured diff report.
//!
//! Configuration is read from `vcs/runner/vtest/differential.yaml`; each
//! `.vr` test may override the matrix via `@backends:` and `@tiers:`
//! comment-line directives.
//!
//! Usage from the CLI:
//!
//!     cargo run -p vtest -- run --differential vcs/specs/L2-standard/shell/multi.vr
//!     cargo run -p vtest -- differential --backend-set unix-only --filter shell

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// One row of the differential matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendDescriptor {
    /// Identifier — e.g. "bash", "zsh", "fish", "pwsh".
    pub name: String,
    /// Absolute path or PATH-relative command of the shell binary.
    pub cmd: String,
    /// Flags prepended to the shell command (e.g. `["-eu"]`).
    pub flags: Vec<String>,
    /// `true` if this backend should be skipped on the current host
    /// (binary missing, platform unsupported, etc.).
    #[serde(default)]
    pub skip_if_missing: bool,
}

/// Tier name — kept as an enum to align with `directive::Tier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    Interpreter,
    Jit,
    Aot,
}

/// Configuration loaded from `differential.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DifferentialConfig {
    pub backends: Vec<BackendDescriptor>,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<Tier>,
    /// Per-test allow-list of expected divergence — pair (test_path, backend)
    /// is recorded with a justification.
    #[serde(default)]
    pub allowed_divergence: Vec<AllowedDivergence>,
}

fn default_tiers() -> Vec<Tier> {
    vec![Tier::Interpreter, Tier::Aot]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedDivergence {
    /// Filename suffix (matched with `ends_with`).
    pub test: String,
    /// Backends whose results may differ.
    pub backends: Vec<String>,
    /// Justification — printed in the report.
    pub reason: String,
}

impl DifferentialConfig {
    /// Default matrix: probe-the-host for each common Unix shell; on
    /// Windows include pwsh.  No filesystem read; suitable when the
    /// project lacks an explicit `differential.yaml`.
    pub fn host_default() -> Self {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let backends = vec![
            BackendDescriptor {
                name: "bash".into(),
                cmd:  "/bin/bash".into(),
                flags: vec!["-eu".into()],
                skip_if_missing: true,
            },
            BackendDescriptor {
                name: "zsh".into(),
                cmd:  "/bin/zsh".into(),
                flags: vec!["-e".into()],
                skip_if_missing: true,
            },
            BackendDescriptor {
                name: "dash".into(),
                cmd:  "/bin/dash".into(),
                flags: vec!["-eu".into()],
                skip_if_missing: true,
            },
            BackendDescriptor {
                name: "fish".into(),
                cmd:  "/usr/bin/fish".into(),
                flags: vec![],
                skip_if_missing: true,
            },
        ];
        #[cfg(target_os = "windows")]
        let backends = vec![
            BackendDescriptor {
                name: "pwsh".into(),
                cmd:  "pwsh".into(),
                flags: vec!["-NoProfile".into()],
                skip_if_missing: true,
            },
            BackendDescriptor {
                name: "cmd".into(),
                cmd:  "cmd.exe".into(),
                flags: vec!["/C".into()],
                skip_if_missing: true,
            },
        ];

        Self {
            backends,
            tiers: default_tiers(),
            allowed_divergence: vec![],
        }
    }

    /// Read `differential.toml` from disk; on read failure or missing
    /// file, return the host default.  TOML is used (not YAML) because
    /// the rest of the runner already depends on the `toml` crate.
    pub fn load_or_default(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => match toml::from_str(&s) {
                Ok(cfg) => cfg,
                Err(_)  => Self::host_default(),
            },
            Err(_) => Self::host_default(),
        }
    }

    /// True iff `path/backend` pair appears in the allowed-divergence list.
    pub fn divergence_allowed(&self, test: &Path, backend: &str) -> Option<&str> {
        let test_str = test.to_string_lossy();
        self.allowed_divergence
            .iter()
            .find(|a| {
                test_str.ends_with(&a.test) && a.backends.iter().any(|b| b == backend)
            })
            .map(|a| a.reason.as_str())
    }
}

// =============================================================================
// Per-execution report
// =============================================================================

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub backend: String,
    pub tier:    Tier,
    pub stdout:  String,
    pub stderr:  String,
    pub exit_code: i32,
    pub duration: Duration,
    pub skipped: bool,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DifferentialReport {
    pub test_path:   PathBuf,
    pub executions:  Vec<ExecutionResult>,
    pub divergences: Vec<Divergence>,
    pub passed:      bool,
}

#[derive(Debug, Clone)]
pub struct Divergence {
    pub a: (String, Tier),
    pub b: (String, Tier),
    pub kind: DivergenceKind,
    pub diff: String,
    pub justified: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceKind {
    ExitCode,
    Stdout,
    Stderr,
}

// =============================================================================
// Runner
// =============================================================================

pub struct DifferentialRunner {
    pub config: DifferentialConfig,
    pub timeout_ms: u64,
}

impl DifferentialRunner {
    pub fn new(config: DifferentialConfig) -> Self {
        Self { config, timeout_ms: 30_000 }
    }

    /// Run the entire matrix on `test_path`.
    pub fn run(&self, test_path: &Path) -> DifferentialReport {
        let mut executions = Vec::new();
        for backend in &self.config.backends {
            for &tier in &self.config.tiers {
                executions.push(self.run_one(test_path, backend, tier));
            }
        }
        let divergences = self.detect_divergences(test_path, &executions);
        let passed = divergences.iter().all(|d| d.justified.is_some());
        DifferentialReport {
            test_path: test_path.to_path_buf(),
            executions,
            divergences,
            passed,
        }
    }

    fn run_one(&self, test_path: &Path, backend: &BackendDescriptor, tier: Tier) -> ExecutionResult {
        let start = std::time::Instant::now();

        // Probe binary existence when skip_if_missing is set.
        if backend.skip_if_missing && !command_available(&backend.cmd) {
            return ExecutionResult {
                backend: backend.name.clone(),
                tier,
                stdout: String::new(),
                stderr: String::new(),
                exit_code: -1,
                duration: start.elapsed(),
                skipped: true,
                skip_reason: Some(format!("{} not found in PATH", backend.cmd)),
            };
        }

        // Build the verum CLI invocation:
        //   verum run --tier <tier> --shell <backend.cmd> <test_path>
        // The `--shell` flag is a hint to ShellContext.shell_program; the
        // actual Verum binary chooses when to invoke the shell.  For
        // pure-Verum tests this knob has no effect — the divergence detector
        // simply sees identical results across backends, which is the win.
        let tier_arg = match tier {
            Tier::Interpreter => "0",
            Tier::Jit         => "1",
            Tier::Aot         => "2",
        };
        let mut cmd = Command::new("verum");
        cmd.args(["run", "--tier", tier_arg, "--shell", &backend.cmd]);
        cmd.arg(test_path);

        match run_with_timeout(cmd, self.timeout_ms) {
            Ok((stdout, stderr, code)) => ExecutionResult {
                backend: backend.name.clone(),
                tier,
                stdout, stderr, exit_code: code,
                duration: start.elapsed(),
                skipped: false,
                skip_reason: None,
            },
            Err(e) => ExecutionResult {
                backend: backend.name.clone(),
                tier,
                stdout: String::new(),
                stderr: format!("execution error: {}", e),
                exit_code: -1,
                duration: start.elapsed(),
                skipped: false,
                skip_reason: None,
            },
        }
    }

    fn detect_divergences(&self, test_path: &Path, exes: &[ExecutionResult]) -> Vec<Divergence> {
        let mut out = Vec::new();
        // Pairwise compare every two non-skipped executions.
        for i in 0..exes.len() {
            if exes[i].skipped { continue; }
            for j in (i + 1)..exes.len() {
                if exes[j].skipped { continue; }
                let a = &exes[i];
                let b = &exes[j];
                if a.exit_code != b.exit_code {
                    out.push(self.divergence(test_path, a, b, DivergenceKind::ExitCode,
                        format!("a={}, b={}", a.exit_code, b.exit_code)));
                }
                if normalise(&a.stdout) != normalise(&b.stdout) {
                    out.push(self.divergence(test_path, a, b, DivergenceKind::Stdout,
                        unified_diff(&a.stdout, &b.stdout)));
                }
            }
        }
        out
    }

    fn divergence(
        &self,
        test_path: &Path,
        a: &ExecutionResult,
        b: &ExecutionResult,
        kind: DivergenceKind,
        diff: String,
    ) -> Divergence {
        // Mark divergence as justified if either backend appears in the
        // allow list for this test.
        let justified = self.config.divergence_allowed(test_path, &a.backend)
            .or_else(|| self.config.divergence_allowed(test_path, &b.backend))
            .map(|s| s.to_string());
        Divergence {
            a: (a.backend.clone(), a.tier),
            b: (b.backend.clone(), b.tier),
            kind,
            diff,
            justified,
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn command_available(cmd: &str) -> bool {
    // Fast path: the path looks absolute → just stat.
    let p = Path::new(cmd);
    if p.is_absolute() {
        return p.exists();
    }
    // Otherwise probe via `which`/`where`.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let probe = Command::new("which").arg(cmd).output();
    #[cfg(target_os = "windows")]
    let probe = Command::new("where").arg(cmd).output();
    match probe {
        Ok(o)  => o.status.success(),
        Err(_) => false,
    }
}

fn run_with_timeout(
    mut cmd: Command,
    timeout_ms: u64,
) -> Result<(String, String, i32), std::io::Error> {
    use std::io::Read;
    use std::process::Stdio;

    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn()?;

    let timeout = Duration::from_millis(timeout_ms);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut h) = child.stdout.take() { let _ = h.read_to_string(&mut stdout); }
                if let Some(mut h) = child.stderr.take() { let _ = h.read_to_string(&mut stderr); }
                return Ok((stdout, stderr, status.code().unwrap_or(-1)));
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("test timed out after {}ms", timeout_ms),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Normalise stdout for comparison: strip trailing whitespace, unify line endings.
fn normalise(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

/// Compact line-diff for divergence reports.
fn unified_diff(a: &str, b: &str) -> String {
    let a_lines: Vec<&str> = a.lines().collect();
    let b_lines: Vec<&str> = b.lines().collect();
    let mut out = String::new();
    let n = a_lines.len().max(b_lines.len());
    for i in 0..n {
        let av = a_lines.get(i).copied().unwrap_or("<EOF>");
        let bv = b_lines.get(i).copied().unwrap_or("<EOF>");
        if av != bv {
            out.push_str(&format!("- {}\n+ {}\n", av, bv));
        }
    }
    out
}

// =============================================================================
// Parse @backends / @tiers directives from a .vr test file
// =============================================================================

#[derive(Debug, Clone, Default)]
pub struct PerTestOverrides {
    pub backends: Option<Vec<String>>,
    pub tiers: Option<Vec<Tier>>,
}

impl PerTestOverrides {
    /// Scan the first ~100 lines of `source` for `@backends:` / `@tiers:`
    /// directives.  Both accept JSON-like list syntax: `[bash, zsh]`.
    pub fn parse(source: &str) -> Self {
        let mut out = PerTestOverrides::default();
        for line in source.lines().take(100) {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("//").map(str::trim_start) {
                if let Some(v) = rest.strip_prefix("@backends:") {
                    out.backends = Some(parse_string_list(v.trim()));
                }
                if let Some(v) = rest.strip_prefix("@tiers:") {
                    out.tiers = Some(
                        parse_string_list(v.trim())
                            .into_iter()
                            .filter_map(|s| match s.as_str() {
                                "0" | "interpreter" => Some(Tier::Interpreter),
                                "1" | "jit"         => Some(Tier::Jit),
                                "2" | "aot"         => Some(Tier::Aot),
                                _ => None,
                            })
                            .collect(),
                    );
                }
            }
        }
        out
    }

    /// Apply per-test overrides on top of a base config.
    pub fn apply(&self, mut base: DifferentialConfig) -> DifferentialConfig {
        if let Some(filter) = &self.backends {
            base.backends.retain(|b| filter.iter().any(|n| n == &b.name));
        }
        if let Some(t) = &self.tiers {
            base.tiers = t.clone();
        }
        base
    }
}

fn parse_string_list(s: &str) -> Vec<String> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    s.split(',')
        .map(|x| x.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

// =============================================================================
// Integration: rendering report as text
// =============================================================================

impl DifferentialReport {
    /// Format the report for terminal output.  Uses no colour codes —
    /// the wider runner adds those at the level it knows about.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "differential test {} — {} divergence(s) across {} executions\n",
            self.test_path.display(),
            self.divergences.len(),
            self.executions.len(),
        ));
        if !self.passed {
            for d in &self.divergences {
                if d.justified.is_some() { continue; }
                out.push_str(&format!(
                    "\n  {:?}: {}({:?}) vs {}({:?})\n",
                    d.kind, d.a.0, d.a.1, d.b.0, d.b.1
                ));
                for line in d.diff.lines().take(20) {
                    out.push_str("    ");
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        out
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_default_has_some_backends() {
        let cfg = DifferentialConfig::host_default();
        assert!(!cfg.backends.is_empty(), "host default backends should not be empty");
    }

    #[test]
    fn parse_overrides_extracts_backend_filter() {
        let src = "// @backends: [bash, zsh]\n// @tiers: [0, 2]\nfn main() {}\n";
        let o = PerTestOverrides::parse(src);
        assert_eq!(o.backends.as_ref().unwrap(), &vec!["bash".to_string(), "zsh".to_string()]);
        assert_eq!(o.tiers.as_ref().unwrap(), &vec![Tier::Interpreter, Tier::Aot]);
    }

    #[test]
    fn divergence_kind_compares_correctly() {
        assert_eq!(DivergenceKind::Stdout, DivergenceKind::Stdout);
        assert_ne!(DivergenceKind::Stdout, DivergenceKind::ExitCode);
    }

    #[test]
    fn normalise_strips_crlf_and_trailing_ws() {
        assert_eq!(normalise("hello\r\n"), "hello");
        assert_eq!(normalise("a\nb\r\n"),  "a\nb");
        assert_eq!(normalise("  trailing  "), "  trailing");
    }

    #[test]
    fn unified_diff_marks_differing_lines() {
        let d = unified_diff("a\nb\nc\n", "a\nx\nc\n");
        assert!(d.contains("- b"), "diff should mark old line: {}", d);
        assert!(d.contains("+ x"), "diff should mark new line: {}", d);
    }

    #[test]
    fn divergence_allowed_matches_test_suffix() {
        let cfg = DifferentialConfig {
            backends: vec![],
            tiers: vec![],
            allowed_divergence: vec![AllowedDivergence {
                test: "shell/quirky.vr".into(),
                backends: vec!["dash".into()],
                reason: "POSIX dash lacks ** glob".into(),
            }],
        };
        let p = Path::new("/some/path/shell/quirky.vr");
        assert_eq!(
            cfg.divergence_allowed(p, "dash"),
            Some("POSIX dash lacks ** glob"),
        );
        assert_eq!(cfg.divergence_allowed(p, "bash"), None);
    }
}
