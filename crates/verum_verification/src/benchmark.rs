//! Continuous benchmarking — head-to-head vs Coq / Lean4 / Isabelle
//! / Agda.
//!
//! ## Goal
//!
//! Establish Verum's quantitative leadership claim with
//! **reproducible** benchmarks across the proof-assistant
//! landscape.  Anyone can re-run the suite and verify the
//! numbers.  No marketing claims, only verifiable measurements.
//!
//! Per benchmark this module captures:
//!
//!   1. **Trusted Computing Base size** — kernel LOC + transitive
//!      trust dependencies.
//!   2. **Compilation / verification speed** — LOC/sec or
//!      theorems/sec on a fixed reference corpus.
//!   3. **Memory peak** — per-theorem peak RSS.
//!   4. **Cross-format export coverage** — how many independent
//!      kernels can re-check a given proof.
//!   5. **Tactic library completeness** — percentage of standard
//!      obligations closed by 1-line tactic invocations.
//!   6. **Trust diversification** — number of independent kernels
//!      that agree on each theorem.
//!   7. **LLM-tactic acceptance rate** — fraction of LLM-proposed
//!      proofs that pass the kernel (only Verum supports this
//!      today; comparable systems baseline at 0%).
//!
//! ## Architectural pattern
//!
//! Same single-trait-boundary pattern as the rest of the
//! integration arc:
//!
//!   * [`BenchmarkSystem`] enum — Verum / Coq / Lean4 / Isabelle /
//!     Agda.
//!   * [`BenchmarkMetric`] enum — typed measurement category.
//!   * [`BenchmarkResult`] — typed per-(system, theorem, metric)
//!     record.
//!   * [`BenchmarkRunner`] trait — single dispatch interface.
//!   * Reference impls: [`MockBenchmarkRunner`] (deterministic, for
//!     tests + the CLI's `--mock` mode), and per-system stubs that
//!     call out to the real tools when available.
//!   * [`ComparisonMatrix`] aggregator — produces head-to-head
//!     matrices across systems.
//!
//! ## V0 contract
//!
//!   * The trait surface, result schema, and aggregator ship now.
//!   * Production runners (running actual `coqc` / `lean` /
//!     `isabelle` / `agda`) are V1+; this module ships the
//!     `MockBenchmarkRunner` for tests and the protocol shape so
//!     downstream runners plug in via the same trait.
//!   * `VerumBenchmarkRunner` runs the local Verum corpus; it ships
//!     today as a reference for comparison-only mode.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use verum_common::Text;

// =============================================================================
// BenchmarkSystem
// =============================================================================

/// One proof-assistant being benchmarked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkSystem {
    Verum,
    Coq,
    Lean4,
    Isabelle,
    Agda,
}

impl BenchmarkSystem {
    pub fn name(self) -> &'static str {
        match self {
            Self::Verum => "verum",
            Self::Coq => "coq",
            Self::Lean4 => "lean4",
            Self::Isabelle => "isabelle",
            Self::Agda => "agda",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "verum" => Some(Self::Verum),
            "coq" | "rocq" => Some(Self::Coq),
            "lean4" | "lean" | "mathlib" => Some(Self::Lean4),
            "isabelle" | "isabelle/hol" | "hol" => Some(Self::Isabelle),
            "agda" => Some(Self::Agda),
            _ => None,
        }
    }

    pub fn all() -> [BenchmarkSystem; 5] {
        [Self::Verum, Self::Coq, Self::Lean4, Self::Isabelle, Self::Agda]
    }
}

// =============================================================================
// BenchmarkMetric — what we measure
// =============================================================================

/// One measurement category.  Each `BenchmarkResult` records the
/// observed value for exactly one metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMetric {
    /// Trusted Computing Base size (kernel + trust deps), in lines.
    /// Lower is better.
    KernelLoc,
    /// Compilation / verification speed (lines per second).  Higher
    /// is better.
    LinesPerSecond,
    /// Theorems verified per second.  Higher is better.
    TheoremsPerSecond,
    /// Per-theorem peak resident-set size in bytes.  Lower is
    /// better.
    PeakRssBytes,
    /// Per-theorem wall-clock duration in milliseconds.  Lower is
    /// better.
    ElapsedMs,
    /// Number of independent kernel formats that successfully
    /// re-check the proof (`verum export` round-trip count).
    /// Higher is better.  Verum target: 4 (Coq + Lean + Isabelle +
    /// Dedukti).
    CrossFormatExports,
    /// Percentage in `[0, 100]` of standard obligations the
    /// system's tactic library closes via 1-line invocations.
    /// Higher is better.
    TacticCoveragePercent,
    /// Number of distinct kernels in the trust circle that agree
    /// on a theorem.  Higher is better.
    TrustDiversificationCount,
    /// Fraction in `[0, 100]` of LLM-proposed proofs that pass the
    /// kernel.  Higher is better.  Only Verum supports this today
    /// (LCF-style fail-closed loop, see #77); comparable systems
    /// baseline at 0%.
    LlmAcceptancePercent,
}

impl BenchmarkMetric {
    pub fn name(self) -> &'static str {
        match self {
            Self::KernelLoc => "kernel_loc",
            Self::LinesPerSecond => "lines_per_second",
            Self::TheoremsPerSecond => "theorems_per_second",
            Self::PeakRssBytes => "peak_rss_bytes",
            Self::ElapsedMs => "elapsed_ms",
            Self::CrossFormatExports => "cross_format_exports",
            Self::TacticCoveragePercent => "tactic_coverage_percent",
            Self::TrustDiversificationCount => "trust_diversification_count",
            Self::LlmAcceptancePercent => "llm_acceptance_percent",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "kernel_loc" => Some(Self::KernelLoc),
            "lines_per_second" => Some(Self::LinesPerSecond),
            "theorems_per_second" => Some(Self::TheoremsPerSecond),
            "peak_rss_bytes" => Some(Self::PeakRssBytes),
            "elapsed_ms" => Some(Self::ElapsedMs),
            "cross_format_exports" => Some(Self::CrossFormatExports),
            "tactic_coverage_percent" => Some(Self::TacticCoveragePercent),
            "trust_diversification_count" => Some(Self::TrustDiversificationCount),
            "llm_acceptance_percent" => Some(Self::LlmAcceptancePercent),
            _ => None,
        }
    }

    /// True iff a higher value is better for this metric.  Used by
    /// the comparison matrix to pick the leader.
    pub fn higher_is_better(self) -> bool {
        match self {
            Self::KernelLoc | Self::PeakRssBytes | Self::ElapsedMs => false,
            Self::LinesPerSecond
            | Self::TheoremsPerSecond
            | Self::CrossFormatExports
            | Self::TacticCoveragePercent
            | Self::TrustDiversificationCount
            | Self::LlmAcceptancePercent => true,
        }
    }

    pub fn all() -> [BenchmarkMetric; 9] {
        [
            Self::KernelLoc,
            Self::LinesPerSecond,
            Self::TheoremsPerSecond,
            Self::PeakRssBytes,
            Self::ElapsedMs,
            Self::CrossFormatExports,
            Self::TacticCoveragePercent,
            Self::TrustDiversificationCount,
            Self::LlmAcceptancePercent,
        ]
    }
}

// =============================================================================
// BenchmarkResult
// =============================================================================

/// One per-(system, suite, theorem, metric) measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub system: BenchmarkSystem,
    /// Suite identifier (e.g. `"mathcomp/basics"`).
    pub suite: Text,
    /// Theorem identifier within the suite.  `None` for
    /// suite-level metrics (e.g. `KernelLoc`).
    pub theorem: Option<Text>,
    pub metric: BenchmarkMetric,
    /// Observed value.  Use floats so `LinesPerSecond` /
    /// `TacticCoveragePercent` etc. work uniformly.
    pub value: f64,
    /// Unix timestamp (seconds) when the measurement was taken.
    pub timestamp: u64,
    /// Optional reproducibility envelope: hash of the input corpus
    /// + tool version.  When set, the same hash + same tool
    /// version should reproduce the same value.
    pub repro_envelope: Option<Text>,
}

impl BenchmarkResult {
    pub fn new(
        system: BenchmarkSystem,
        suite: impl Into<Text>,
        metric: BenchmarkMetric,
        value: f64,
    ) -> Self {
        Self {
            system,
            suite: suite.into(),
            theorem: None,
            metric,
            value,
            timestamp: now_secs(),
            repro_envelope: None,
        }
    }

    pub fn with_theorem(mut self, theorem: impl Into<Text>) -> Self {
        self.theorem = Some(theorem.into());
        self
    }

    pub fn with_envelope(mut self, env: impl Into<Text>) -> Self {
        self.repro_envelope = Some(env.into());
        self
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// =============================================================================
// BenchmarkRunner trait
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum BenchmarkError {
    ToolMissing(Text),
    Transport(Text),
    SuiteParse(Text),
    Other(Text),
}

impl std::fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolMissing(t) => write!(f, "tool missing: {}", t.as_str()),
            Self::Transport(t) => write!(f, "transport: {}", t.as_str()),
            Self::SuiteParse(t) => write!(f, "suite parse: {}", t.as_str()),
            Self::Other(t) => write!(f, "{}", t.as_str()),
        }
    }
}

impl std::error::Error for BenchmarkError {}

/// Single dispatch interface for a per-system benchmark runner.
pub trait BenchmarkRunner: std::fmt::Debug + Send + Sync {
    fn system(&self) -> BenchmarkSystem;

    /// True iff the underlying tool is available in this
    /// environment (e.g. `coqc` is on PATH for `CoqRunner`).
    fn is_available(&self) -> bool;

    /// Run the configured suite and emit one or more results.
    fn run(&self, suite: &BenchmarkSuite) -> Result<Vec<BenchmarkResult>, BenchmarkError>;
}

// =============================================================================
// BenchmarkSuite — input description
// =============================================================================

/// Description of a benchmark suite.  Runners interpret this
/// uniformly: a suite name, a list of theorem identifiers, and an
/// optional repro-envelope (hash of the input corpus).  Concrete
/// runners may map theorem ids to per-system source files via a
/// configured prefix (e.g. `mathcomp/basics → coq/mathcomp/basics.v`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkSuite {
    pub name: Text,
    pub theorems: Vec<Text>,
    pub repro_envelope: Option<Text>,
}

impl BenchmarkSuite {
    pub fn new(name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            theorems: Vec::new(),
            repro_envelope: None,
        }
    }

    pub fn add_theorem(mut self, name: impl Into<Text>) -> Self {
        self.theorems.push(name.into());
        self
    }

    pub fn with_envelope(mut self, env: impl Into<Text>) -> Self {
        self.repro_envelope = Some(env.into());
        self
    }
}

// =============================================================================
// MockBenchmarkRunner — deterministic, test-friendly
// =============================================================================

/// Mock runner that emits canned results for every theorem in the
/// suite.  Used for tests + the CLI's `--mock` mode.
#[derive(Debug, Clone)]
pub struct MockBenchmarkRunner {
    pub system: BenchmarkSystem,
    /// Per-metric canned values returned for every theorem.
    pub canned: BTreeMap<BenchmarkMetric, f64>,
    pub available: bool,
}

impl MockBenchmarkRunner {
    pub fn new(system: BenchmarkSystem) -> Self {
        Self {
            system,
            canned: BTreeMap::new(),
            available: true,
        }
    }

    pub fn with(mut self, metric: BenchmarkMetric, value: f64) -> Self {
        self.canned.insert(metric, value);
        self
    }

    pub fn unavailable(mut self) -> Self {
        self.available = false;
        self
    }
}

impl BenchmarkRunner for MockBenchmarkRunner {
    fn system(&self) -> BenchmarkSystem {
        self.system
    }

    fn is_available(&self) -> bool {
        self.available
    }

    fn run(&self, suite: &BenchmarkSuite) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        if !self.available {
            return Err(BenchmarkError::ToolMissing(Text::from(format!(
                "{} runner not available",
                self.system.name()
            ))));
        }
        let mut out = Vec::new();
        // Suite-level metrics (e.g. KernelLoc) emitted once per suite.
        for (metric, value) in &self.canned {
            if matches!(
                metric,
                BenchmarkMetric::KernelLoc
                    | BenchmarkMetric::LinesPerSecond
                    | BenchmarkMetric::TacticCoveragePercent
                    | BenchmarkMetric::LlmAcceptancePercent
                    | BenchmarkMetric::CrossFormatExports
                    | BenchmarkMetric::TrustDiversificationCount
            ) {
                let mut r = BenchmarkResult::new(
                    self.system,
                    suite.name.clone(),
                    *metric,
                    *value,
                );
                if let Some(env) = &suite.repro_envelope {
                    r = r.with_envelope(env.clone());
                }
                out.push(r);
            }
        }
        // Per-theorem metrics.
        for thm in &suite.theorems {
            for (metric, value) in &self.canned {
                if matches!(
                    metric,
                    BenchmarkMetric::ElapsedMs
                        | BenchmarkMetric::PeakRssBytes
                        | BenchmarkMetric::TheoremsPerSecond
                ) {
                    let mut r = BenchmarkResult::new(
                        self.system,
                        suite.name.clone(),
                        *metric,
                        *value,
                    )
                    .with_theorem(thm.clone());
                    if let Some(env) = &suite.repro_envelope {
                        r = r.with_envelope(env.clone());
                    }
                    out.push(r);
                }
            }
        }
        Ok(out)
    }
}

// =============================================================================
// ProductionBenchmarkRunner — real tool invocation (#94 hardening)
// =============================================================================
//
// V0 shipped only `MockBenchmarkRunner`, returning canned values
// derived from the documented-landscape claims.  Hardening: real
// runners that:
//
//   * Detect tool presence by invoking `<cmd> <version_flag>` and
//     checking the exit code (no PATH gymnastics — `Command::new`
//     resolves through PATH itself, surfacing `NotFound` cleanly).
//   * Run `<cmd> <theorem-source>` per theorem, captures
//     `elapsed_ms` via `Instant::now()`, exit-status as
//     pass/fail, and per-theorem stdout-tail as detail.
//   * Return `BenchmarkError::ToolMissing` when the tool isn't on
//     PATH; the CLI's matrix logic already handles that.
//
// Runners are generic over `(command, version_flag, source_extension)`
// so the same type is reused across systems.  Per-system
// constructors below pin the conventions (`coqc` / `lean` /
// `isabelle process` / `agda` / `verum`).

/// Production runner that invokes a real foreign tool per theorem.
#[derive(Debug, Clone)]
pub struct ProductionBenchmarkRunner {
    system: BenchmarkSystem,
    /// Executable to run (resolved via PATH).
    command: Text,
    /// Extra arguments prepended to every invocation (e.g.
    /// `["process"]` for `isabelle process <session>`).
    leading_args: Vec<Text>,
    /// Argument that prints the tool's version (e.g. `"--version"`)
    /// — used by `is_available` to verify the tool exists and is
    /// callable.
    version_arg: Text,
    /// Directory holding `<theorem>.<extension>` source files.
    source_dir: std::path::PathBuf,
    /// Source-file extension (e.g. `"v"` for Coq, `"lean"` for
    /// Lean4).  Empty means the theorem id IS the full filename.
    source_extension: Text,
    /// Per-theorem wall-clock deadline.  Exceeding kills the
    /// process and emits a timeout result.  V1 — currently the
    /// runner waits unconditionally; a future Wall-clock-budget
    /// extension lands here.
    pub timeout_secs: u64,
}

impl ProductionBenchmarkRunner {
    /// Build a production runner for a generic system.  Per-system
    /// helpers below ([`coq_production_runner`] etc.) pin the
    /// conventional command names so callers don't have to.
    pub fn new(
        system: BenchmarkSystem,
        command: impl Into<Text>,
        source_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        Self {
            system,
            command: command.into(),
            leading_args: Vec::new(),
            version_arg: Text::from("--version"),
            source_dir: source_dir.into(),
            source_extension: Text::from(""),
            timeout_secs: 600,
        }
    }

    pub fn with_version_arg(mut self, arg: impl Into<Text>) -> Self {
        self.version_arg = arg.into();
        self
    }

    pub fn with_leading_args(mut self, args: &[&str]) -> Self {
        self.leading_args = args.iter().map(|s| Text::from(*s)).collect();
        self
    }

    pub fn with_extension(mut self, ext: impl Into<Text>) -> Self {
        self.source_extension = ext.into();
        self
    }

    /// Resolve a theorem id to its on-disk source path.
    fn source_path_for(&self, theorem: &str) -> std::path::PathBuf {
        let ext = self.source_extension.as_str();
        if ext.is_empty() {
            self.source_dir.join(theorem)
        } else {
            self.source_dir.join(format!("{}.{}", theorem, ext))
        }
    }
}

impl BenchmarkRunner for ProductionBenchmarkRunner {
    fn system(&self) -> BenchmarkSystem {
        self.system
    }

    fn is_available(&self) -> bool {
        // `Command::new(cmd).arg(version_arg).output()` returns
        // `Err(io::Error)` (kind == NotFound) when `cmd` is missing
        // from PATH.  Any successful spawn with a 0 exit code
        // (or a 0/non-zero status if the tool prints version to
        // stderr — Lean4's `lean --version` exits 0; Coq's
        // `coqc -v` ditto) is sufficient.
        let mut cmd = std::process::Command::new(self.command.as_str());
        for a in &self.leading_args {
            cmd.arg(a.as_str());
        }
        cmd.arg(self.version_arg.as_str());
        match cmd.output() {
            Ok(_) => true,
            Err(_) => false,
        }
    }

    fn run(&self, suite: &BenchmarkSuite) -> Result<Vec<BenchmarkResult>, BenchmarkError> {
        if !self.is_available() {
            return Err(BenchmarkError::ToolMissing(Text::from(format!(
                "{} runner: `{}` not on PATH",
                self.system.name(),
                self.command.as_str()
            ))));
        }
        let mut out: Vec<BenchmarkResult> = Vec::new();
        let mut total_elapsed_ms: f64 = 0.0;
        let mut completed: u64 = 0;
        for thm in &suite.theorems {
            let source = self.source_path_for(thm.as_str());
            let mut cmd = std::process::Command::new(self.command.as_str());
            for a in &self.leading_args {
                cmd.arg(a.as_str());
            }
            cmd.arg(&source);
            let started = std::time::Instant::now();
            let outcome = cmd.output();
            let elapsed_ms = started.elapsed().as_millis() as f64;
            match outcome {
                Ok(o) if o.status.success() => {
                    total_elapsed_ms += elapsed_ms;
                    completed += 1;
                    let mut r = BenchmarkResult::new(
                        self.system,
                        suite.name.clone(),
                        BenchmarkMetric::ElapsedMs,
                        elapsed_ms,
                    )
                    .with_theorem(thm.clone());
                    if let Some(env) = &suite.repro_envelope {
                        r = r.with_envelope(env.clone());
                    }
                    out.push(r);
                }
                Ok(o) => {
                    return Err(BenchmarkError::Other(Text::from(format!(
                        "{} `{}` exited with non-zero status {} in {}ms; stderr tail: {}",
                        self.system.name(),
                        thm.as_str(),
                        o.status,
                        elapsed_ms,
                        truncate_for_diag(&String::from_utf8_lossy(&o.stderr), 200)
                    ))));
                }
                Err(e) => {
                    return Err(BenchmarkError::ToolMissing(Text::from(format!(
                        "{} `{}` failed to spawn: {}",
                        self.system.name(),
                        thm.as_str(),
                        e
                    ))));
                }
            }
        }
        // Suite-level aggregate: theorems_per_second.
        if total_elapsed_ms > 0.0 {
            let tps = (completed as f64) / (total_elapsed_ms / 1000.0);
            let mut r = BenchmarkResult::new(
                self.system,
                suite.name.clone(),
                BenchmarkMetric::TheoremsPerSecond,
                tps,
            );
            if let Some(env) = &suite.repro_envelope {
                r = r.with_envelope(env.clone());
            }
            out.push(r);
        }
        Ok(out)
    }
}

fn truncate_for_diag(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// `coqc <theorem>.v`.
pub fn coq_production_runner(
    source_dir: impl Into<std::path::PathBuf>,
) -> ProductionBenchmarkRunner {
    ProductionBenchmarkRunner::new(BenchmarkSystem::Coq, "coqc", source_dir)
        .with_extension("v")
        .with_version_arg("--version")
}

/// `lean <theorem>.lean`.
pub fn lean4_production_runner(
    source_dir: impl Into<std::path::PathBuf>,
) -> ProductionBenchmarkRunner {
    ProductionBenchmarkRunner::new(BenchmarkSystem::Lean4, "lean", source_dir)
        .with_extension("lean")
        .with_version_arg("--version")
}

/// `isabelle process <theorem>` (the theorem id is interpreted as a
/// session name; no extension).
pub fn isabelle_production_runner(
    source_dir: impl Into<std::path::PathBuf>,
) -> ProductionBenchmarkRunner {
    ProductionBenchmarkRunner::new(BenchmarkSystem::Isabelle, "isabelle", source_dir)
        .with_leading_args(&["process"])
        .with_extension("thy")
        .with_version_arg("version")
}

/// `agda --no-libraries <theorem>.agda`.
pub fn agda_production_runner(
    source_dir: impl Into<std::path::PathBuf>,
) -> ProductionBenchmarkRunner {
    ProductionBenchmarkRunner::new(BenchmarkSystem::Agda, "agda", source_dir)
        .with_leading_args(&["--no-libraries"])
        .with_extension("agda")
        .with_version_arg("--version")
}

/// `verum verify <theorem>.vr`.
pub fn verum_production_runner(
    source_dir: impl Into<std::path::PathBuf>,
) -> ProductionBenchmarkRunner {
    ProductionBenchmarkRunner::new(BenchmarkSystem::Verum, "verum", source_dir)
        .with_leading_args(&["verify"])
        .with_extension("vr")
        .with_version_arg("--version")
}

// =============================================================================
// runner_for — per-system reference dispatcher
// =============================================================================

/// Pick a reference runner for a system.  V0 ships
/// `MockBenchmarkRunner` for every system (deterministic, no real
/// tool invocation).  V1+ swaps in production runners that call
/// out to the actual tools when available; the trait surface is
/// unchanged.
pub fn mock_runner_for(system: BenchmarkSystem) -> MockBenchmarkRunner {
    let mut r = MockBenchmarkRunner::new(system);
    // Canned baseline values reflecting the documented landscape
    // claims — these are not measurements, they are placeholders
    // for the protocol shape.  V1 production runners replace
    // every entry with real-tool measurements.
    let (kernel, lps, llm) = match system {
        BenchmarkSystem::Verum => (5_000.0, 50_000.0, 65.0),
        BenchmarkSystem::Coq => (200_000.0, 20_000.0, 0.0),
        BenchmarkSystem::Lean4 => (50_000.0, 30_000.0, 0.0),
        BenchmarkSystem::Isabelle => (10_000.0, 25_000.0, 0.0),
        BenchmarkSystem::Agda => (30_000.0, 15_000.0, 0.0),
    };
    r = r
        .with(BenchmarkMetric::KernelLoc, kernel)
        .with(BenchmarkMetric::LinesPerSecond, lps)
        .with(BenchmarkMetric::LlmAcceptancePercent, llm);
    let exports = match system {
        BenchmarkSystem::Verum => 4.0, // Coq + Lean + Isabelle + Dedukti
        _ => 1.0,                      // each foreign system re-checks itself only
    };
    r = r.with(BenchmarkMetric::CrossFormatExports, exports);
    r
}

// =============================================================================
// ComparisonMatrix — head-to-head aggregator
// =============================================================================

/// Aggregator for cross-system comparison output.  Indexed by
/// `(metric, system) → value` and exposes leader detection per
/// metric.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComparisonMatrix {
    pub suite: Text,
    pub by_metric_and_system: BTreeMap<(BenchmarkMetric, BenchmarkSystem), f64>,
}

impl ComparisonMatrix {
    pub fn new(suite: impl Into<Text>) -> Self {
        Self {
            suite: suite.into(),
            by_metric_and_system: BTreeMap::new(),
        }
    }

    /// Ingest a batch of results.  Per `(metric, system)` the
    /// stored value is the last one ingested — duplicates from a
    /// per-theorem metric are AVERAGED across theorems before
    /// storage.  See [`Self::ingest_aggregated`] for that path.
    pub fn ingest(&mut self, result: BenchmarkResult) {
        self.by_metric_and_system
            .insert((result.metric, result.system), result.value);
    }

    /// Ingest a batch and average per-theorem metrics across the
    /// theorems in the same suite.  Suite-level metrics are
    /// untouched.
    pub fn ingest_aggregated(&mut self, results: &[BenchmarkResult]) {
        let mut sums: BTreeMap<(BenchmarkMetric, BenchmarkSystem), (f64, usize)> =
            BTreeMap::new();
        for r in results {
            let key = (r.metric, r.system);
            let entry = sums.entry(key).or_insert((0.0, 0));
            entry.0 += r.value;
            entry.1 += 1;
        }
        for (key, (sum, count)) in sums {
            let avg = if count > 0 { sum / count as f64 } else { 0.0 };
            self.by_metric_and_system.insert(key, avg);
        }
    }

    /// Return the system with the best value for the given metric.
    /// `None` when no system has data.  Ties broken by alphabetical
    /// system order (determinism).
    pub fn leader(&self, metric: BenchmarkMetric) -> Option<BenchmarkSystem> {
        let mut best: Option<(BenchmarkSystem, f64)> = None;
        for (&(m, s), &v) in &self.by_metric_and_system {
            if m != metric {
                continue;
            }
            best = match best {
                None => Some((s, v)),
                Some((bs, bv)) => {
                    let metric_higher = metric.higher_is_better();
                    let s_better = if metric_higher { v > bv } else { v < bv };
                    if s_better {
                        Some((s, v))
                    } else {
                        Some((bs, bv))
                    }
                }
            };
        }
        best.map(|(s, _)| s)
    }

    /// Render a Markdown table.  Rows are metrics, columns are
    /// systems, leader cells are decorated with `⭐`.
    pub fn to_markdown(&self) -> Text {
        let metrics_in_use: Vec<BenchmarkMetric> = {
            let mut s: std::collections::BTreeSet<BenchmarkMetric> =
                std::collections::BTreeSet::new();
            for (m, _) in self.by_metric_and_system.keys() {
                s.insert(*m);
            }
            s.into_iter().collect()
        };
        let systems_in_use: Vec<BenchmarkSystem> = {
            let mut s: std::collections::BTreeSet<BenchmarkSystem> =
                std::collections::BTreeSet::new();
            for (_, sys) in self.by_metric_and_system.keys() {
                s.insert(*sys);
            }
            s.into_iter().collect()
        };
        let mut out = String::new();
        out.push_str(&format!(
            "# Benchmark comparison — `{}`\n\n",
            self.suite.as_str()
        ));
        // Header.
        out.push_str("| Metric |");
        for s in &systems_in_use {
            out.push_str(&format!(" {} |", s.name()));
        }
        out.push('\n');
        out.push_str("|---|");
        for _ in &systems_in_use {
            out.push_str("---|");
        }
        out.push('\n');
        // Rows.
        for m in &metrics_in_use {
            out.push_str(&format!("| `{}` |", m.name()));
            let leader = self.leader(*m);
            for s in &systems_in_use {
                match self.by_metric_and_system.get(&(*m, *s)) {
                    Some(v) => {
                        let marker = if leader == Some(*s) { " ⭐" } else { "" };
                        out.push_str(&format!(" {}{} |", format_value(*m, *v), marker));
                    }
                    None => out.push_str(" — |"),
                }
            }
            out.push('\n');
        }
        Text::from(out)
    }
}

fn format_value(metric: BenchmarkMetric, value: f64) -> String {
    match metric {
        BenchmarkMetric::KernelLoc => format!("{} LOC", format_thousands(value as i64)),
        BenchmarkMetric::LinesPerSecond => format!(
            "{} LOC/s",
            format_thousands(value as i64)
        ),
        BenchmarkMetric::TheoremsPerSecond => format!("{:.1} thm/s", value),
        BenchmarkMetric::PeakRssBytes => format!("{:.1} MB", value / (1024.0 * 1024.0)),
        BenchmarkMetric::ElapsedMs => format!("{:.1} ms", value),
        BenchmarkMetric::CrossFormatExports => format!("{}", value as i64),
        BenchmarkMetric::TacticCoveragePercent => format!("{:.1}%", value),
        BenchmarkMetric::TrustDiversificationCount => format!("{}", value as i64),
        BenchmarkMetric::LlmAcceptancePercent => format!("{:.1}%", value),
    }
}

fn format_thousands(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_suite() -> BenchmarkSuite {
        BenchmarkSuite::new("mathcomp/basics")
            .add_theorem("addnC")
            .add_theorem("addn0")
            .add_theorem("addnA")
    }

    // ----- BenchmarkSystem -----

    #[test]
    fn system_round_trip_and_aliases() {
        for s in BenchmarkSystem::all() {
            assert_eq!(BenchmarkSystem::from_name(s.name()), Some(s));
        }
        assert_eq!(BenchmarkSystem::from_name("rocq"), Some(BenchmarkSystem::Coq));
        assert_eq!(
            BenchmarkSystem::from_name("mathlib"),
            Some(BenchmarkSystem::Lean4)
        );
        assert_eq!(
            BenchmarkSystem::from_name("hol"),
            Some(BenchmarkSystem::Isabelle)
        );
        assert_eq!(BenchmarkSystem::from_name(""), None);
    }

    #[test]
    fn all_systems_have_distinct_names() {
        let names: std::collections::BTreeSet<&str> =
            BenchmarkSystem::all().iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), 5);
    }

    // ----- BenchmarkMetric -----

    #[test]
    fn metric_round_trip() {
        for m in BenchmarkMetric::all() {
            assert_eq!(BenchmarkMetric::from_name(m.name()), Some(m));
        }
    }

    #[test]
    fn higher_is_better_partition() {
        let lower: Vec<BenchmarkMetric> = BenchmarkMetric::all()
            .iter()
            .copied()
            .filter(|m| !m.higher_is_better())
            .collect();
        // Lower-is-better: KernelLoc, PeakRssBytes, ElapsedMs.
        assert_eq!(lower.len(), 3);
        assert!(lower.contains(&BenchmarkMetric::KernelLoc));
        assert!(lower.contains(&BenchmarkMetric::PeakRssBytes));
        assert!(lower.contains(&BenchmarkMetric::ElapsedMs));
    }

    #[test]
    fn nine_canonical_metrics_present() {
        // Pin the metric inventory; updating the canonical set
        // must update the docs + simplifier + CI scripts too.
        assert_eq!(BenchmarkMetric::all().len(), 9);
    }

    // ----- BenchmarkResult builder -----

    #[test]
    fn benchmark_result_builder() {
        let r = BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "x",
            BenchmarkMetric::KernelLoc,
            5000.0,
        )
        .with_theorem("foo")
        .with_envelope("blake3-of-corpus");
        assert_eq!(r.theorem.as_ref().unwrap().as_str(), "foo");
        assert_eq!(r.repro_envelope.as_ref().unwrap().as_str(), "blake3-of-corpus");
    }

    #[test]
    fn benchmark_result_serde_round_trip() {
        let r = BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "x",
            BenchmarkMetric::LinesPerSecond,
            50_000.0,
        );
        let s = serde_json::to_string(&r).unwrap();
        let back: BenchmarkResult = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    // ----- MockBenchmarkRunner -----

    #[test]
    fn mock_runner_emits_results_for_every_theorem() {
        let runner = MockBenchmarkRunner::new(BenchmarkSystem::Verum)
            .with(BenchmarkMetric::ElapsedMs, 10.0)
            .with(BenchmarkMetric::KernelLoc, 5000.0);
        let suite = fixture_suite();
        let results = runner.run(&suite).unwrap();
        // Per-theorem ElapsedMs × 3 theorems + 1 suite-level KernelLoc = 4.
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn mock_runner_unavailable_returns_tool_missing() {
        let runner = MockBenchmarkRunner::new(BenchmarkSystem::Coq).unavailable();
        let suite = fixture_suite();
        match runner.run(&suite) {
            Err(BenchmarkError::ToolMissing(_)) => {}
            other => panic!("expected ToolMissing, got {:?}", other),
        }
    }

    #[test]
    fn mock_runner_propagates_repro_envelope() {
        let runner = MockBenchmarkRunner::new(BenchmarkSystem::Verum)
            .with(BenchmarkMetric::KernelLoc, 5000.0);
        let suite = fixture_suite().with_envelope("hash-abc");
        let results = runner.run(&suite).unwrap();
        assert!(results.iter().all(|r| {
            r.repro_envelope.as_ref().map(|t| t.as_str()) == Some("hash-abc")
        }));
    }

    // ----- mock_runner_for -----

    #[test]
    fn mock_runner_for_returns_correct_system() {
        for s in BenchmarkSystem::all() {
            assert_eq!(mock_runner_for(s).system(), s);
        }
    }

    #[test]
    fn mock_runner_for_verum_has_smallest_kernel() {
        let verum_loc = mock_runner_for(BenchmarkSystem::Verum)
            .canned
            .get(&BenchmarkMetric::KernelLoc)
            .copied()
            .unwrap();
        for s in [
            BenchmarkSystem::Coq,
            BenchmarkSystem::Lean4,
            BenchmarkSystem::Isabelle,
            BenchmarkSystem::Agda,
        ] {
            let other = mock_runner_for(s)
                .canned
                .get(&BenchmarkMetric::KernelLoc)
                .copied()
                .unwrap();
            assert!(verum_loc < other, "verum kernel must be smaller than {}", s.name());
        }
    }

    #[test]
    fn mock_runner_for_only_verum_has_nonzero_llm_acceptance() {
        for s in BenchmarkSystem::all() {
            let v = mock_runner_for(s)
                .canned
                .get(&BenchmarkMetric::LlmAcceptancePercent)
                .copied()
                .unwrap();
            if s == BenchmarkSystem::Verum {
                assert!(v > 0.0);
            } else {
                assert_eq!(v, 0.0);
            }
        }
    }

    // ----- ComparisonMatrix -----

    #[test]
    fn comparison_matrix_ingests_results() {
        let mut m = ComparisonMatrix::new("suite");
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "suite",
            BenchmarkMetric::KernelLoc,
            5000.0,
        ));
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Coq,
            "suite",
            BenchmarkMetric::KernelLoc,
            200_000.0,
        ));
        assert_eq!(m.by_metric_and_system.len(), 2);
    }

    #[test]
    fn comparison_matrix_leader_lower_is_better() {
        let mut m = ComparisonMatrix::new("suite");
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "s",
            BenchmarkMetric::KernelLoc,
            5000.0,
        ));
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Coq,
            "s",
            BenchmarkMetric::KernelLoc,
            200_000.0,
        ));
        // Lower KernelLoc wins.
        assert_eq!(m.leader(BenchmarkMetric::KernelLoc), Some(BenchmarkSystem::Verum));
    }

    #[test]
    fn comparison_matrix_leader_higher_is_better() {
        let mut m = ComparisonMatrix::new("suite");
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "s",
            BenchmarkMetric::LinesPerSecond,
            50_000.0,
        ));
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Coq,
            "s",
            BenchmarkMetric::LinesPerSecond,
            20_000.0,
        ));
        assert_eq!(
            m.leader(BenchmarkMetric::LinesPerSecond),
            Some(BenchmarkSystem::Verum)
        );
    }

    #[test]
    fn comparison_matrix_leader_none_when_empty() {
        let m = ComparisonMatrix::new("suite");
        assert_eq!(m.leader(BenchmarkMetric::KernelLoc), None);
    }

    #[test]
    fn comparison_matrix_ingest_aggregated_averages_per_theorem() {
        let mut m = ComparisonMatrix::new("suite");
        let results = vec![
            BenchmarkResult::new(
                BenchmarkSystem::Verum,
                "s",
                BenchmarkMetric::ElapsedMs,
                10.0,
            ),
            BenchmarkResult::new(
                BenchmarkSystem::Verum,
                "s",
                BenchmarkMetric::ElapsedMs,
                30.0,
            ),
        ];
        m.ingest_aggregated(&results);
        // (10 + 30) / 2 = 20.
        assert_eq!(
            m.by_metric_and_system
                .get(&(BenchmarkMetric::ElapsedMs, BenchmarkSystem::Verum)),
            Some(&20.0)
        );
    }

    #[test]
    fn comparison_matrix_to_markdown_includes_leader_marker() {
        let mut m = ComparisonMatrix::new("suite");
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Verum,
            "s",
            BenchmarkMetric::KernelLoc,
            5000.0,
        ));
        m.ingest(BenchmarkResult::new(
            BenchmarkSystem::Coq,
            "s",
            BenchmarkMetric::KernelLoc,
            200_000.0,
        ));
        let md = m.to_markdown();
        let s = md.as_str();
        assert!(s.contains("# Benchmark comparison"));
        assert!(s.contains("kernel_loc"));
        assert!(s.contains("⭐")); // Verum is the leader for kernel_loc
        assert!(s.contains("verum"));
        assert!(s.contains("coq"));
    }

    // ----- Acceptance pin -----

    #[test]
    fn task_83_seven_canonical_benchmarks_categories() {
        // Every category called out in #83's description must be
        // representable as a metric.
        let categories: &[BenchmarkMetric] = &[
            BenchmarkMetric::KernelLoc,                  // §1
            BenchmarkMetric::LinesPerSecond,             // §2
            BenchmarkMetric::PeakRssBytes,               // §3
            BenchmarkMetric::CrossFormatExports,         // §4
            BenchmarkMetric::TacticCoveragePercent,      // §5
            BenchmarkMetric::TrustDiversificationCount,  // §6
            BenchmarkMetric::LlmAcceptancePercent,       // §7
        ];
        for c in categories {
            assert!(BenchmarkMetric::from_name(c.name()).is_some());
        }
    }

    #[test]
    fn task_83_verum_leads_kernel_size_and_llm_acceptance() {
        // Pin the documented landscape claims as a
        // shape-of-leadership test.  Real-tool runners will
        // produce the actual numbers; the protocol shape is what
        // we lock in here.
        let mut m = ComparisonMatrix::new("suite");
        for s in BenchmarkSystem::all() {
            let runner = mock_runner_for(s);
            for (metric, value) in &runner.canned {
                m.ingest(BenchmarkResult::new(s, "suite", *metric, *value));
            }
        }
        assert_eq!(m.leader(BenchmarkMetric::KernelLoc), Some(BenchmarkSystem::Verum));
        assert_eq!(
            m.leader(BenchmarkMetric::LlmAcceptancePercent),
            Some(BenchmarkSystem::Verum)
        );
        assert_eq!(
            m.leader(BenchmarkMetric::CrossFormatExports),
            Some(BenchmarkSystem::Verum)
        );
    }

    // =========================================================================
    // ProductionBenchmarkRunner (#94 hardening)
    // =========================================================================

    #[test]
    fn production_runner_factories_pin_command_and_extension() {
        let dir = std::path::PathBuf::from("/tmp/whatever");
        let coq = coq_production_runner(&dir);
        assert_eq!(coq.system, BenchmarkSystem::Coq);
        assert_eq!(coq.command.as_str(), "coqc");
        assert_eq!(coq.source_extension.as_str(), "v");

        let lean = lean4_production_runner(&dir);
        assert_eq!(lean.command.as_str(), "lean");
        assert_eq!(lean.source_extension.as_str(), "lean");

        let isa = isabelle_production_runner(&dir);
        assert_eq!(isa.command.as_str(), "isabelle");
        assert_eq!(isa.leading_args.len(), 1);
        assert_eq!(isa.leading_args[0].as_str(), "process");

        let agda = agda_production_runner(&dir);
        assert_eq!(agda.command.as_str(), "agda");

        let verum = verum_production_runner(&dir);
        assert_eq!(verum.command.as_str(), "verum");
    }

    #[test]
    fn production_runner_source_path_uses_extension() {
        let dir = std::path::PathBuf::from("/some/dir");
        let r = coq_production_runner(&dir);
        let p = r.source_path_for("foo");
        assert_eq!(p, std::path::PathBuf::from("/some/dir/foo.v"));
    }

    #[test]
    fn production_runner_source_path_no_extension_uses_id_directly() {
        let dir = std::path::PathBuf::from("/some/dir");
        let r = ProductionBenchmarkRunner::new(BenchmarkSystem::Verum, "verum", &dir);
        let p = r.source_path_for("session_X");
        assert_eq!(p, std::path::PathBuf::from("/some/dir/session_X"));
    }

    #[test]
    fn production_runner_unavailable_when_command_missing() {
        let dir = std::path::PathBuf::from("/tmp");
        let r = ProductionBenchmarkRunner::new(
            BenchmarkSystem::Coq,
            "definitely_not_a_real_command_e7a92f",
            &dir,
        );
        assert!(!r.is_available());
        let suite = BenchmarkSuite::new("s").add_theorem("t1");
        match r.run(&suite) {
            Err(BenchmarkError::ToolMissing(t)) => {
                assert!(t.as_str().contains("definitely_not_a_real_command_e7a92f"));
            }
            other => panic!("expected ToolMissing, got {:?}", other),
        }
    }

    #[test]
    fn production_runner_runs_real_tool_against_tempfile() {
        // Use `/bin/echo` as a stand-in for a tool that always
        // succeeds — the production runner protocol is the same:
        // version-check passes, per-theorem invocation exits 0,
        // elapsed_ms gets recorded.
        let tmpdir = tempfile::tempdir().unwrap();
        let thm_path = tmpdir.path().join("toy.txt");
        std::fs::write(&thm_path, "stub").unwrap();
        let r = ProductionBenchmarkRunner::new(
            BenchmarkSystem::Verum,
            "echo",
            tmpdir.path(),
        )
        .with_extension("txt");
        assert!(r.is_available());
        let suite = BenchmarkSuite::new("s").add_theorem("toy");
        let results = r.run(&suite).unwrap();
        // One ElapsedMs result + one TheoremsPerSecond aggregate.
        let elapsed_count = results
            .iter()
            .filter(|r| r.metric == BenchmarkMetric::ElapsedMs)
            .count();
        assert_eq!(elapsed_count, 1);
        let tps_count = results
            .iter()
            .filter(|r| r.metric == BenchmarkMetric::TheoremsPerSecond)
            .count();
        assert_eq!(tps_count, 1);
    }

    #[test]
    fn production_runner_propagates_nonzero_exit() {
        // `false` always exits 1 — the runner must surface this as
        // a structured error rather than silently coding 0.
        let tmpdir = tempfile::tempdir().unwrap();
        let r = ProductionBenchmarkRunner::new(
            BenchmarkSystem::Verum,
            "false",
            tmpdir.path(),
        );
        if !r.is_available() {
            // `false` is missing on some build sandboxes; skip
            // gracefully rather than failing the suite.
            return;
        }
        let suite = BenchmarkSuite::new("s").add_theorem("nope");
        let err = r.run(&suite).unwrap_err();
        match err {
            BenchmarkError::Other(t) => {
                assert!(t.as_str().contains("non-zero status"));
            }
            other => panic!("expected Other (non-zero exit), got {:?}", other),
        }
    }

    #[test]
    fn task_94_real_runners_replace_canned_values_when_tools_present() {
        // Acceptance: when the underlying tool is on PATH, the
        // production runner does NOT use the canned landscape
        // values — it measures `elapsed_ms` from `Instant::now()`.
        // We pin the *protocol* using `echo` as a stand-in (always
        // available across CI); the same code path applies to
        // coqc / lean / isabelle / agda / verum when those binaries
        // are present.
        let tmpdir = tempfile::tempdir().unwrap();
        let thm_path = tmpdir.path().join("p.txt");
        std::fs::write(&thm_path, "x").unwrap();
        let r = ProductionBenchmarkRunner::new(
            BenchmarkSystem::Lean4,
            "echo",
            tmpdir.path(),
        )
        .with_extension("txt");
        let suite = BenchmarkSuite::new("acceptance").add_theorem("p");
        let results = r.run(&suite).unwrap();
        let elapsed = results
            .iter()
            .find(|r| r.metric == BenchmarkMetric::ElapsedMs)
            .expect("ElapsedMs must be emitted");
        // The mock for Lean4 has no per-theorem ElapsedMs entry;
        // the production runner does, with a measured value.
        assert!(elapsed.value >= 0.0);
        assert_eq!(elapsed.theorem.as_ref().unwrap().as_str(), "p");
    }
}
