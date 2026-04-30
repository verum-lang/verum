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

    /// Canonical foreign-system handle.  Default implementation
    /// converts [`format`](Self::format) via
    /// [`ExportFormat::to_foreign_system`].  Lets consumers dispatch
    /// by typed enum without going through the format → ID mapping.
    fn foreign_system(&self) -> verum_kernel::foreign_system::ForeignSystem {
        self.format().to_foreign_system()
    }
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

// =============================================================================
// Docker backend (#149 / MSFS-L4.15)
// =============================================================================
//
// Turns the cross-format gate from observability (host needs coqc / lean
// installed) into a load-bearing CI gate (host needs only Docker).  Each
// foreign tool runs inside its canonical container image, mounted on a
// host directory of emitted .v / .lean files.
//
// The backend is selected via the `--docker` flag on the audit gate and
// (independently) via the `VERUM_FOREIGN_TOOL_BACKEND` env var (`docker`
// or `native`; default `native`).  Image names are configurable per-
// format via env (`VERUM_DOCKER_IMAGE_COQ`, `VERUM_DOCKER_IMAGE_LEAN`,
// etc.) so CI can pin to a specific tag.

/// Per-format Docker image configuration.  Defaults pin known-good
/// images for reproducible CI; override via env vars.
#[derive(Debug, Clone)]
pub struct DockerCheckerConfig {
    /// Image name + tag (e.g., `coqorg/coq:8.18.0-flambda`).
    pub image: String,
    /// Tool inside the container (e.g., `coqc`, `lean`).
    pub tool_in_container: String,
    /// Extra args passed to the tool (e.g., `["-q"]` for coq).
    pub tool_args: Vec<String>,
    /// Container mount point for the host directory holding the
    /// emitted certificate file (default `/work`).
    pub mount_point: String,
    /// Per-invocation timeout in seconds (default 120).
    pub timeout_secs: u64,
}

impl DockerCheckerConfig {
    /// Coq config — `coqorg/coq:8.18.0-flambda` runs `coqc` inside
    /// `/work`.  The image's user is `coq` with HOME at `/home/coq`,
    /// but `coqc` doesn't need write access outside the mount point.
    pub fn coq_default() -> Self {
        Self {
            image: std::env::var("VERUM_DOCKER_IMAGE_COQ")
                .unwrap_or_else(|_| "coqorg/coq:8.18.0-flambda".to_string()),
            tool_in_container: "coqc".to_string(),
            tool_args: vec!["-q".to_string()],
            mount_point: "/work".to_string(),
            timeout_secs: 120,
        }
    }

    /// Lean 4 config — `leanprovercommunity/lean4:4.5.0` runs `lean`.
    /// The image ships an `elan`-managed lean toolchain at the pinned
    /// version.
    pub fn lean_default() -> Self {
        Self {
            image: std::env::var("VERUM_DOCKER_IMAGE_LEAN")
                .unwrap_or_else(|_| "leanprovercommunity/lean4:4.5.0".to_string()),
            tool_in_container: "lean".to_string(),
            tool_args: Vec::new(),
            mount_point: "/work".to_string(),
            timeout_secs: 120,
        }
    }
}

/// Helper: invoke `docker run --rm -v <host_dir>:<mount> <image>
/// <tool> <args...> <mount>/<filename>`.  Captures exit/stderr/stdout
/// and lifts to a [`CheckResult`].
fn run_docker_tool(
    config: &DockerCheckerConfig,
    file_path: &Path,
    excerpt_bytes: usize,
) -> CheckResult {
    if !is_tool_on_path("docker") {
        return CheckResult::ToolMissing {
            install_hint: "install Docker (https://docs.docker.com/get-docker/) and ensure the daemon is running"
                .to_string(),
        };
    }
    // Resolve host directory + filename — Docker mounts directories,
    // not individual files, so we mount the parent dir read-only and
    // pass the basename to the tool.
    let host_dir = match file_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => match std::env::current_dir() {
            Ok(d) => d,
            Err(e) => {
                return CheckResult::RunnerError {
                    reason: format!("cannot resolve cwd for docker mount: {}", e),
                };
            }
        },
    };
    let host_dir_str = host_dir.to_string_lossy();
    let basename = match file_path.file_name() {
        Some(b) => b.to_string_lossy().into_owned(),
        None => {
            return CheckResult::RunnerError {
                reason: format!("file path has no basename: {:?}", file_path),
            };
        }
    };
    let mount_arg = format!("{}:{}:ro", host_dir_str, config.mount_point);
    let in_container_path = format!("{}/{}", config.mount_point, basename);
    // Build full argv: `docker run --rm -v <mount> <image> <tool> <tool_args...> <file>`.
    let mut argv: Vec<String> = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--network=none".to_string(), // hermetic; no network access
        "-v".to_string(),
        mount_arg,
        "-w".to_string(),
        config.mount_point.clone(),
        config.image.clone(),
        config.tool_in_container.clone(),
    ];
    argv.extend(config.tool_args.iter().cloned());
    argv.push(in_container_path);

    let started = Instant::now();
    let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let output = match Command::new("docker").args(&argv_refs).output() {
        Ok(o) => o,
        Err(e) => {
            return CheckResult::RunnerError {
                reason: format!("docker spawn failed: {}", e),
            };
        }
    };
    let elapsed = started.elapsed();
    let stdout_excerpt = excerpt_utf8(&output.stdout, excerpt_bytes);
    let stderr_excerpt = excerpt_utf8(&output.stderr, excerpt_bytes);
    if output.status.success() {
        CheckResult::Passed {
            tool_version: format!("{} (docker)", config.image),
            elapsed,
            stdout_excerpt,
        }
    } else {
        let exit_code = output.status.code().unwrap_or(-1);
        // Distinguish docker-infrastructure failures from real
        // foreign-tool failures.  When docker itself errors before the
        // tool runs (daemon down, image pull failure, etc.) we surface
        // `ToolMissing` so the gate doesn't conflate "infrastructure
        // unhealthy" with "the proof was rejected by coqc/lean".
        if is_docker_infrastructure_error(&stderr_excerpt) {
            return CheckResult::ToolMissing {
                install_hint: format!(
                    "docker invocation failed before the foreign tool ran (image={}). \
                     Check that the docker daemon is reachable and the image is available. \
                     Original stderr: {}",
                    config.image, stderr_excerpt,
                ),
            };
        }
        CheckResult::Failed {
            exit_code,
            stderr_excerpt,
            stdout_excerpt,
        }
    }
}

/// Recognise stderr substrings that indicate docker itself failed
/// before invoking the foreign tool — daemon-not-running, image-pull
/// failure, permission errors on the socket, etc.  Used to surface
/// these as `ToolMissing` (infrastructure issue) rather than `Failed`
/// (proof failure).
fn is_docker_infrastructure_error(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("cannot connect to the docker daemon")
        || s.contains("failed to connect to the docker api")
        || s.contains("docker daemon")
        || s.contains("error response from daemon")
        || s.contains("permission denied while trying to connect to the docker daemon socket")
        || s.contains("docker.sock")
        || s.contains("pull access denied")
        || s.contains("manifest unknown")
        || s.contains("no such image")
        || s.contains("image not found")
}

/// Coq via Docker.
#[derive(Debug, Clone)]
pub struct DockerCoqChecker {
    config: DockerCheckerConfig,
}

impl DockerCoqChecker {
    pub fn new() -> Self {
        Self {
            config: DockerCheckerConfig::coq_default(),
        }
    }
    pub fn with_config(config: DockerCheckerConfig) -> Self {
        Self { config }
    }
}

impl Default for DockerCoqChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl ForeignSystemChecker for DockerCoqChecker {
    fn format(&self) -> ExportFormat {
        ExportFormat::Coq
    }
    fn is_available(&self) -> bool {
        is_tool_on_path("docker")
    }
    fn install_hint(&self) -> &'static str {
        "install Docker; the Coq image is pulled on first run"
    }
    fn check_file(&self, path: &Path) -> CheckResult {
        run_docker_tool(&self.config, path, 4096)
    }
}

/// Lean 4 via Docker.
#[derive(Debug, Clone)]
pub struct DockerLeanChecker {
    config: DockerCheckerConfig,
}

impl DockerLeanChecker {
    pub fn new() -> Self {
        Self {
            config: DockerCheckerConfig::lean_default(),
        }
    }
    pub fn with_config(config: DockerCheckerConfig) -> Self {
        Self { config }
    }
}

impl Default for DockerLeanChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl ForeignSystemChecker for DockerLeanChecker {
    fn format(&self) -> ExportFormat {
        ExportFormat::Lean4
    }
    fn is_available(&self) -> bool {
        is_tool_on_path("docker")
    }
    fn install_hint(&self) -> &'static str {
        "install Docker; the Lean 4 image is pulled on first run"
    }
    fn check_file(&self, path: &Path) -> CheckResult {
        run_docker_tool(&self.config, path, 4096)
    }
}

/// Backend selection — native PATH-resolved tool vs Docker container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckerBackend {
    /// Use the host's PATH-resolved tool.  Default.
    Native,
    /// Run the foreign tool inside its canonical Docker image.
    Docker,
}

impl CheckerBackend {
    /// Read backend selection from `VERUM_FOREIGN_TOOL_BACKEND`
    /// environment variable.  Defaults to `Native`.  Recognises:
    ///   * `docker` / `Docker` / `DOCKER` → `CheckerBackend::Docker`
    ///   * anything else (incl. unset) → `CheckerBackend::Native`
    pub fn from_env() -> Self {
        match std::env::var("VERUM_FOREIGN_TOOL_BACKEND")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "docker" => CheckerBackend::Docker,
            _ => CheckerBackend::Native,
        }
    }
}

/// Return the canonical checker for a format under a given backend.
/// Backend-aware sibling of [`checker_for`].
pub fn checker_for_backend(
    format: ExportFormat,
    backend: CheckerBackend,
) -> Option<Box<dyn ForeignSystemChecker>> {
    match (format, backend) {
        (ExportFormat::Coq, CheckerBackend::Docker) => Some(Box::new(DockerCoqChecker::new())),
        (ExportFormat::Lean4, CheckerBackend::Docker) => Some(Box::new(DockerLeanChecker::new())),
        // Isabelle / Dedukti Docker variants left as future work; fall
        // back to native for now.
        (format, _) => checker_for(format),
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

    // =========================================================================
    // Docker backend tests (#149 / MSFS-L4.15)
    // =========================================================================

    /// Static mutex serialising env-var-mutating tests in this module.
    /// Cargo runs unit tests in parallel by default; without this the
    /// VERUM_DOCKER_IMAGE_COQ / VERUM_FOREIGN_TOOL_BACKEND tests
    /// interleave and observe stale values from other threads.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn docker_coq_default_config_is_pinned() {
        let _guard = ENV_MUTEX.lock().unwrap();
        // Clear potential override from a prior test before checking
        // the default-tag invariant.
        unsafe {
            std::env::remove_var("VERUM_DOCKER_IMAGE_COQ");
        }
        let cfg = DockerCheckerConfig::coq_default();
        assert!(cfg.image.starts_with("coqorg/coq:"));
        assert_eq!(cfg.tool_in_container, "coqc");
        assert!(cfg.tool_args.iter().any(|a| a == "-q"));
        assert_eq!(cfg.mount_point, "/work");
    }

    #[test]
    fn docker_lean_default_config_is_pinned() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::remove_var("VERUM_DOCKER_IMAGE_LEAN");
        }
        let cfg = DockerCheckerConfig::lean_default();
        assert!(cfg.image.contains("lean4"));
        assert_eq!(cfg.tool_in_container, "lean");
        assert_eq!(cfg.mount_point, "/work");
    }

    #[test]
    fn docker_image_env_override_takes_precedence() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::set_var("VERUM_DOCKER_IMAGE_COQ", "custom/coq:test-tag");
        }
        let cfg = DockerCheckerConfig::coq_default();
        assert_eq!(cfg.image, "custom/coq:test-tag");
        unsafe {
            std::env::remove_var("VERUM_DOCKER_IMAGE_COQ");
        }
    }

    #[test]
    fn checker_backend_from_env_default_and_docker() {
        let _guard = ENV_MUTEX.lock().unwrap();
        unsafe {
            std::env::remove_var("VERUM_FOREIGN_TOOL_BACKEND");
        }
        assert_eq!(CheckerBackend::from_env(), CheckerBackend::Native);
        unsafe {
            std::env::set_var("VERUM_FOREIGN_TOOL_BACKEND", "docker");
        }
        assert_eq!(CheckerBackend::from_env(), CheckerBackend::Docker);
        unsafe {
            std::env::set_var("VERUM_FOREIGN_TOOL_BACKEND", "DOCKER");
        }
        assert_eq!(CheckerBackend::from_env(), CheckerBackend::Docker);
        unsafe {
            std::env::set_var("VERUM_FOREIGN_TOOL_BACKEND", "garbage_value");
        }
        // Unknown value falls back to Native (defensive).
        assert_eq!(CheckerBackend::from_env(), CheckerBackend::Native);
        unsafe {
            std::env::remove_var("VERUM_FOREIGN_TOOL_BACKEND");
        }
    }

    #[test]
    fn checker_for_backend_native_matches_checker_for() {
        // Backend-aware dispatch under Native should produce the
        // same checker as the legacy `checker_for` for every format.
        for f in [
            ExportFormat::Coq,
            ExportFormat::Lean4,
            ExportFormat::Isabelle,
            ExportFormat::Dedukti,
        ] {
            let native_via_legacy = checker_for(f);
            let native_via_backend = checker_for_backend(f, CheckerBackend::Native);
            // Both must agree on Some/None — actual format identity is
            // checked via the checker's format() method.
            assert_eq!(native_via_legacy.is_some(), native_via_backend.is_some());
            if let (Some(a), Some(b)) = (native_via_legacy, native_via_backend) {
                assert_eq!(a.format(), b.format());
            }
        }
    }

    #[test]
    fn docker_infrastructure_error_detection_covers_common_cases() {
        // Daemon-down / socket / pull-failure / image-missing patterns
        // surface as ToolMissing, not Failed (infrastructure ≠ proof
        // rejection).
        assert!(is_docker_infrastructure_error(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock"
        ));
        assert!(is_docker_infrastructure_error(
            "failed to connect to the docker API at unix:///path/docker.sock"
        ));
        assert!(is_docker_infrastructure_error(
            "Error response from daemon: pull access denied for nonexistent/image"
        ));
        assert!(is_docker_infrastructure_error("manifest unknown"));
        assert!(is_docker_infrastructure_error("Unable to find image 'foo:bar' locally\n\nError: No such image: foo:bar"));
        // Real proof-tool errors must NOT match (false positives would
        // mis-classify legitimate failures).
        assert!(!is_docker_infrastructure_error("Error: type mismatch"));
        assert!(!is_docker_infrastructure_error("syntax error: unexpected token"));
        assert!(!is_docker_infrastructure_error("Coq < error: definition refused"));
    }

    #[test]
    fn checker_for_backend_docker_returns_docker_variants_for_coq_and_lean() {
        let coq = checker_for_backend(ExportFormat::Coq, CheckerBackend::Docker).unwrap();
        assert_eq!(coq.format(), ExportFormat::Coq);
        let lean = checker_for_backend(ExportFormat::Lean4, CheckerBackend::Docker).unwrap();
        assert_eq!(lean.format(), ExportFormat::Lean4);
        // Isabelle / Dedukti Docker variants not yet wired — fall back
        // to native (still some).  Pin the contract.
        let isa = checker_for_backend(ExportFormat::Isabelle, CheckerBackend::Docker);
        assert!(isa.is_some());
    }
}
