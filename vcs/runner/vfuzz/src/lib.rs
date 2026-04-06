//! VCS Fuzzer (vfuzz) - Property-based fuzz testing for the Verum compiler
//!
//! This crate provides comprehensive fuzzing infrastructure for testing the
//! Verum compiler's robustness. It integrates with the VCS (Verum Compliance
//! Suite) to provide:
//!
//! - **Random Program Generation**: Create valid/invalid Verum programs
//! - **Differential Testing**: Compare behavior across execution tiers (Tier 0 vs Tier 3)
//! - **Input Mutation**: Apply smart mutations to seed programs
//! - **Test Case Shrinking**: Minimize failing inputs to minimal reproducers
//! - **Coverage Tracking**: Track interesting test cases for corpus evolution
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────────┐
//! │                              vfuzz                                        │
//! ├──────────────────────────────────────────────────────────────────────────┤
//! │                                                                           │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐     │
//! │  │  Generator  │  │   Mutator   │  │   Corpus    │  │   Shrink    │     │
//! │  │             │  │             │  │   Manager   │  │   Engine    │     │
//! │  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘     │
//! │         │                │                │                │            │
//! │         └────────────────┼────────────────┘                │            │
//! │                          ▼                                 │            │
//! │                 ┌─────────────────┐                        │            │
//! │                 │   Fuzz Engine   │◄───────────────────────┘            │
//! │                 └────────┬────────┘                                     │
//! │                          │                                              │
//! │                          ▼                                              │
//! │         ┌────────────────────────────────┐                              │
//! │         │       Execution Harness         │                              │
//! │         │  ┌──────────┐  ┌──────────┐   │                              │
//! │         │  │  Tier 0  │  │  Tier 3  │   │                              │
//! │         │  │  (Interp)│  │  (Native)│   │                              │
//! │         │  └──────────┘  └──────────┘   │                              │
//! │         └────────────────────────────────┘                              │
//! │                          │                                              │
//! │                          ▼                                              │
//! │                 ┌─────────────────┐                                     │
//! │                 │   Result Cmp    │                                     │
//! │                 └─────────────────┘                                     │
//! │                          │                                              │
//! │           ┌──────────────┼──────────────┐                               │
//! │           ▼              ▼              ▼                               │
//! │     ┌──────────┐  ┌──────────┐  ┌──────────┐                           │
//! │     │   Pass   │  │   Fail   │  │  Crash   │                           │
//! │     └──────────┘  └──────────┘  └──────────┘                           │
//! │                          │              │                               │
//! │                          └──────────────┘                               │
//! │                                 │                                       │
//! │                                 ▼                                       │
//! │                        ┌─────────────────┐                              │
//! │                        │     Shrink      │                              │
//! │                        └─────────────────┘                              │
//! │                                 │                                       │
//! │                                 ▼                                       │
//! │                        ┌─────────────────┐                              │
//! │                        │  Bug Report     │                              │
//! │                        └─────────────────┘                              │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use verum_vfuzz::{FuzzEngine, FuzzConfig, CorpusManager};
//!
//! // Create fuzzer with default config
//! let config = FuzzConfig::default();
//! let mut engine = FuzzEngine::new(config).unwrap();
//!
//! // Load seed corpus
//! engine.load_seeds("vcs/fuzz/seeds").unwrap();
//!
//! // Run fuzzing campaign
//! let stats = engine.run(10_000).unwrap();
//! println!("Found {} issues", stats.issues_found);
//! ```

// VCS fuzzer infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]
#![allow(missing_docs)]
#![allow(deprecated)]

pub mod campaign;
pub mod corpus;
pub mod coverage;
pub mod generator;
pub mod generators;
pub mod modes;
pub mod mutator;
pub mod oracle;
pub mod property;
pub mod report;
pub mod shrink;
pub mod triage;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use campaign::{
    // Checkpointing
    CampaignCheckpoint,
    CampaignConfig,
    CampaignPhase,
    CampaignState,
    CampaignType,
    CorpusMetadata,
    CoverageSettings,
    EnergyConfig,
    // Energy scheduling
    EnergyScheduler,
    EnergyStats,
    GeneratorSettings,
    GeneratorStrategy,
    MutatorSettings,
    OracleSettings,
    ParallelConfig,
    // Parallel coordination
    ParallelCoordinator,
    PowerSchedule,
    QueueEntry,
    ReportFormat,
    ReportingSettings,
    // Seed corpus management
    SeedCorpus,
    SeedCorpusConfig,
    SeedEntry,
    SeedFormat,
    SeedSource,
    ShrinkingSettings,
    ShrinkingStrategy,
    StopConditions,
    StopReason,
    TargetComponent,
    WorkItem,
    WorkType,
    WorkerPhase,
    WorkerState,
};
pub use corpus::{Corpus, CorpusEntry, CorpusManager, CoverageInfo};
pub use coverage::{
    AstNodeCoverage,
    AstNodeCoverageReport,
    AstNodeStats,
    // Enhanced coverage types
    BranchCoverage,
    BranchCoverageReport,
    BranchId,
    BranchState,
    CoverageBitmap,
    CoverageScheduler,
    CoverageStats,
    CoverageTracker,
    ErrorCodeCoverage,
    ErrorCodeCoverageReport,
    ErrorCodeStats,
    ErrorSeverity,
    GlobalCoverage,
    SmtTheory,
    SmtTheoryCoverage,
    SmtTheoryCoverageReport,
    SmtTheoryStats,
    SourceCoverage,
    SourceCoverageReport,
    UnifiedCoverage,
    UnifiedCoverageReport,
};
pub use generator::{Generator, GeneratorConfig, GeneratorKind};
pub use generators::UnifiedGenerator;
pub use mutator::{MutationStrategy, Mutator, MutatorConfig};
pub use oracle::{
    // New oracle types
    CrashOracle,
    CrashOracleConfig,
    CrashType,
    CrashViolation,
    DifferentialOracle,
    DifferentialResult,
    ExecutionTier,
    MemorySafetyConfig,
    MemorySafetyOracle,
    MemorySafetyViolation,
    OracleRunner,
    OracleRunnerConfig,
    OracleViolation,
    SmtOracle,
    SmtOracleConfig,
    SmtViolation,
    SmtViolationKind,
    TierMismatch,
    TierResult,
    TimeoutOracle,
    TimeoutOracleConfig,
    TimeoutPhase,
    TimeoutViolation,
    TypeSafetyConfig,
    TypeSafetyKind,
    TypeSafetyOracle,
    TypeSafetyViolation,
};
pub use property::{
    AbsorptionProperties,
    AssociativityProperties,
    CommutativityProperties,
    CompilerProperties,
    DistributivityProperties,
    // Extended runner
    ExtendedPropertyRunner,
    // Property categories
    IdempotencyProperties,
    IdentityProperties,
    InvolutionProperties,
    MonotonicityProperties,
    Property,
    PropertyCategory,
    PropertyCombinator,
    PropertyFailure,
    PropertyResult,
    PropertyRunner,
    PropertyStats,
    PropertyTest,
    PropertyTestResult,
    RoundtripProperties,
    boolean_properties,
    collection_properties,
    // Property generators
    numeric_properties,
};
pub use shrink::{
    // Enhanced shrinkers
    AstAwareShrinker,
    AstNode,
    AstNodeKind,
    AstShrinkConfig,
    AstShrinkStats,
    DeltaDebugConfig,
    DeltaDebugStats,
    DeltaDebugger,
    DeltaUnit,
    HierarchicalConfig,
    HierarchicalShrinker,
    HierarchicalStats,
    ShrinkConfig,
    ShrinkLevel,
    ShrinkResult,
    ShrinkStats,
    ShrinkStrategy,
    Shrinker,
};
pub use triage::{
    // Auto-categorization
    AutoCategorizer,
    BaselineCrash,
    BugCategory,
    BugReport,
    CategoryRule,
    CrashBaseline,
    CrashClass,
    // Enhanced deduplication
    CrashDeduplicator,
    CrashInfo,
    CrashSignature,
    CrashTriager,
    DedupConfig,
    DedupStats,
    PanicKind,
    Priority,
    RegressionConfig,
    RegressionCrash,
    // Regression detection
    RegressionDetector,
    RegressionReport,
    RegressionSummary,
    RuntimeCrashKind,
    Severity,
    SeverityAssessment,
    // Severity classification
    SeverityClassifier,
    SeverityConfig,
    SeverityFactor,
    SignatureBucket,
    StackFrame,
    TriageStats,
};

/// Errors that can occur during fuzzing
#[derive(Debug, Error)]
pub enum FuzzError {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Execution error
    #[error("Execution error: {0}")]
    Execution(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result type for fuzzing operations
pub type FuzzResult<T> = Result<T, FuzzError>;

/// Configuration for the fuzzing engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzConfig {
    /// Number of iterations to run (0 = infinite)
    pub iterations: usize,
    /// Timeout per test case in milliseconds
    pub timeout_ms: u64,
    /// Number of parallel workers
    pub workers: usize,
    /// Directory for crash artifacts
    pub crash_dir: PathBuf,
    /// Directory for the corpus
    pub corpus_dir: PathBuf,
    /// Whether to minimize crashing inputs
    pub minimize: bool,
    /// Maximum program size in bytes
    pub max_program_size: usize,
    /// Random seed (None for random)
    pub seed: Option<u64>,
    /// Enable differential testing (Tier 0 vs Tier 3)
    pub differential: bool,
    /// Maximum depth for generated programs
    pub max_depth: usize,
    /// Maximum statements per function
    pub max_statements: usize,
    /// Mutation probability (0.0 - 1.0)
    pub mutation_prob: f64,
    /// Generation probability (0.0 - 1.0)
    pub generation_prob: f64,
    /// Whether to print progress
    pub verbose: bool,
    /// Save all interesting inputs
    pub save_interesting: bool,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            iterations: 0, // Infinite
            timeout_ms: 10_000,
            workers: num_cpus(),
            crash_dir: PathBuf::from("vcs/fuzz/crashes"),
            corpus_dir: PathBuf::from("vcs/fuzz/corpus"),
            minimize: true,
            max_program_size: 100_000,
            seed: None,
            differential: true,
            max_depth: 10,
            max_statements: 50,
            mutation_prob: 0.8,
            generation_prob: 0.2,
            verbose: false,
            save_interesting: true,
        }
    }
}

/// Statistics from a fuzzing campaign
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FuzzStats {
    /// Total iterations run
    pub iterations: usize,
    /// Total time spent in seconds
    pub duration_secs: f64,
    /// Number of issues found
    pub issues_found: usize,
    /// Number of crashes
    pub crashes: usize,
    /// Number of unique crashes (by stack trace hash)
    pub unique_crashes: usize,
    /// Number of differential bugs
    pub differential_bugs: usize,
    /// Number of timeouts
    pub timeouts: usize,
    /// Number of interesting inputs found
    pub interesting_inputs: usize,
    /// Corpus size
    pub corpus_size: usize,
    /// Programs per second
    pub throughput: f64,
    /// Peak memory usage in bytes
    pub peak_memory: usize,
    /// Coverage percentage (if available)
    pub coverage_pct: Option<f64>,
}

/// Type of issue found during fuzzing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum IssueKind {
    /// Compiler crashed (panic, segfault, etc.)
    Crash(CrashKind),
    /// Different results between tiers
    DifferentialMismatch,
    /// Timeout during execution
    Timeout,
    /// Memory safety issue (CBGR violation)
    MemorySafety,
    /// Type system unsoundness
    TypeUnsoundness,
    /// Verification failure
    VerificationFailure,
}

/// Type of crash
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CrashKind {
    /// Rust panic
    Panic,
    /// Segmentation fault
    Segfault,
    /// Stack overflow
    StackOverflow,
    /// Out of memory
    OutOfMemory,
    /// Assertion failure
    Assertion,
    /// Unknown crash
    Unknown,
}

/// Information about a found issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    /// Unique identifier (hash of input)
    pub id: String,
    /// The input that triggered the issue
    pub input: String,
    /// Type of issue
    pub kind: IssueKind,
    /// Error message or stack trace
    pub message: String,
    /// Minimized input (if available)
    pub minimized: Option<String>,
    /// Timestamp when found
    pub timestamp: String,
    /// Tier where issue was found
    pub tier: Option<Tier>,
    /// Additional metadata
    pub metadata: serde_json::Value,
}

impl Issue {
    /// Create a new issue
    pub fn new(input: &str, kind: IssueKind, message: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let hash = hasher.finalize();
        let id = hex::encode(&hash[..8]);

        Self {
            id,
            input: input.to_string(),
            kind,
            message: message.to_string(),
            minimized: None,
            timestamp: chrono::Utc::now().to_rfc3339(),
            tier: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Save issue to a file
    pub fn save(&self, dir: &Path) -> FuzzResult<PathBuf> {
        std::fs::create_dir_all(dir)?;

        // Save the input
        let input_path = dir.join(format!("{}.vr", self.id));
        std::fs::write(&input_path, &self.input)?;

        // Save the metadata
        let meta_path = dir.join(format!("{}.json", self.id));
        let meta = serde_json::to_string_pretty(self)?;
        std::fs::write(&meta_path, meta)?;

        Ok(input_path)
    }
}

/// Execution tier for differential testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Tier 0: Interpreter (most safe, slowest)
    Tier0,
    /// Tier 1: Basic JIT
    Tier1,
    /// Tier 2: Optimized JIT
    Tier2,
    /// Tier 3: AOT native (fastest, must match Tier 0)
    Tier3,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tier::Tier0 => write!(f, "Tier0 (Interpreter)"),
            Tier::Tier1 => write!(f, "Tier1 (JIT-Base)"),
            Tier::Tier2 => write!(f, "Tier2 (JIT-Opt)"),
            Tier::Tier3 => write!(f, "Tier3 (AOT)"),
        }
    }
}

/// Result of executing a program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Whether execution succeeded
    pub success: bool,
    /// Output value (if any)
    pub output: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Execution time in nanoseconds
    pub duration_ns: u64,
    /// Memory used in bytes
    pub memory_bytes: usize,
    /// Tier used for execution
    pub tier: Tier,
}

/// The main fuzzing engine
pub struct FuzzEngine {
    config: FuzzConfig,
    generator: Generator,
    mutator: Mutator,
    shrinker: Shrinker,
    corpus: CorpusManager,
    rng: ChaCha8Rng,
    stats: FuzzStats,
    seen_hashes: HashSet<String>,
    issues: Vec<Issue>,
    /// Running flag (public for Ctrl+C handler)
    pub running: Arc<AtomicBool>,
    iteration_count: Arc<AtomicUsize>,
}

impl FuzzEngine {
    /// Create a new fuzzing engine with the given configuration
    pub fn new(config: FuzzConfig) -> FuzzResult<Self> {
        let seed = config.seed.unwrap_or_else(rand::random);
        let rng = ChaCha8Rng::seed_from_u64(seed);

        let generator_config = GeneratorConfig {
            max_depth: config.max_depth,
            max_statements: config.max_statements,
            ..Default::default()
        };

        let mutator_config = MutatorConfig {
            mutation_rate: config.mutation_prob,
            ..Default::default()
        };

        let shrink_config = ShrinkConfig {
            max_iterations: 1000,
            ..Default::default()
        };

        // Create directories
        std::fs::create_dir_all(&config.crash_dir)?;
        std::fs::create_dir_all(&config.corpus_dir)?;

        Ok(Self {
            generator: Generator::new(generator_config),
            mutator: Mutator::new(mutator_config),
            shrinker: Shrinker::new(shrink_config),
            corpus: CorpusManager::new(&config.corpus_dir)?,
            config,
            rng,
            stats: FuzzStats::default(),
            seen_hashes: HashSet::new(),
            issues: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            iteration_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Load seed corpus from a directory
    pub fn load_seeds<P: AsRef<Path>>(&mut self, dir: P) -> FuzzResult<usize> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        for entry in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                let content = std::fs::read_to_string(path)?;
                self.corpus.add(&content, None);
                count += 1;
            }
        }

        if self.config.verbose {
            eprintln!("Loaded {} seed files from {:?}", count, dir);
        }

        Ok(count)
    }

    /// Run the fuzzing campaign
    pub fn run(&mut self, iterations: usize) -> FuzzResult<FuzzStats> {
        let start = Instant::now();
        self.running.store(true, Ordering::SeqCst);
        self.iteration_count.store(0, Ordering::SeqCst);

        let target_iterations = if iterations == 0 {
            usize::MAX
        } else {
            iterations
        };

        while self.running.load(Ordering::SeqCst)
            && self.iteration_count.load(Ordering::SeqCst) < target_iterations
        {
            self.run_iteration()?;
            self.iteration_count.fetch_add(1, Ordering::SeqCst);

            // Progress reporting
            let count = self.iteration_count.load(Ordering::SeqCst);
            if self.config.verbose && count % 1000 == 0 {
                let elapsed = start.elapsed().as_secs_f64();
                let throughput = count as f64 / elapsed;
                eprintln!(
                    "[{}] iter: {}, crashes: {}, diff: {}, throughput: {:.1}/s",
                    chrono::Local::now().format("%H:%M:%S"),
                    count,
                    self.stats.crashes,
                    self.stats.differential_bugs,
                    throughput
                );
            }
        }

        self.stats.iterations = self.iteration_count.load(Ordering::SeqCst);
        self.stats.duration_secs = start.elapsed().as_secs_f64();
        self.stats.throughput = self.stats.iterations as f64 / self.stats.duration_secs;
        self.stats.corpus_size = self.corpus.len();
        self.stats.issues_found =
            self.stats.crashes + self.stats.differential_bugs + self.stats.timeouts;

        Ok(self.stats.clone())
    }

    /// Run a single fuzzing iteration
    fn run_iteration(&mut self) -> FuzzResult<()> {
        // Decide whether to generate or mutate
        let input =
            if self.corpus.is_empty() || self.rng.random::<f64>() < self.config.generation_prob {
                // Generate a new program
                self.generator.generate(&mut self.rng)
            } else {
                // Mutate an existing corpus entry
                let base = self.corpus.pick(&mut self.rng);
                self.mutator.mutate(&base, &mut self.rng)
            };

        // Skip if too large
        if input.len() > self.config.max_program_size {
            return Ok(());
        }

        // Check for duplicates
        let hash = self.hash_input(&input);
        if self.seen_hashes.contains(&hash) {
            return Ok(());
        }
        self.seen_hashes.insert(hash.clone());

        // Execute and check for issues
        let result = self.execute_and_check(&input)?;

        if let Some(issue) = result {
            self.handle_issue(issue)?;
        }

        Ok(())
    }

    /// Execute the input and check for issues
    fn execute_and_check(&mut self, input: &str) -> FuzzResult<Option<Issue>> {
        // Simulate execution for now (actual implementation would use verum_compiler)
        // This is a placeholder that demonstrates the structure

        // Check for obvious crash patterns (simplified)
        if input.contains("panic!") || input.contains("unreachable!") {
            return Ok(Some(Issue::new(
                input,
                IssueKind::Crash(CrashKind::Panic),
                "Explicit panic in code",
            )));
        }

        // Differential testing
        if self.config.differential {
            let tier0_result = self.execute_tier(input, Tier::Tier0)?;
            let tier3_result = self.execute_tier(input, Tier::Tier3)?;

            if tier0_result.success && tier3_result.success {
                if tier0_result.output != tier3_result.output {
                    let mut issue = Issue::new(
                        input,
                        IssueKind::DifferentialMismatch,
                        &format!(
                            "Tier0 output: {:?}, Tier3 output: {:?}",
                            tier0_result.output, tier3_result.output
                        ),
                    );
                    issue.metadata = serde_json::json!({
                        "tier0": tier0_result,
                        "tier3": tier3_result,
                    });
                    return Ok(Some(issue));
                }
            }
        }

        // Check if input is interesting (new coverage)
        if self.is_interesting(input) {
            self.stats.interesting_inputs += 1;
            self.corpus.add(input, None);

            if self.config.save_interesting {
                let path = self
                    .config
                    .corpus_dir
                    .join(format!("interesting_{}.vr", self.stats.interesting_inputs));
                std::fs::write(path, input)?;
            }
        }

        Ok(None)
    }

    /// Execute on a specific tier
    fn execute_tier(&self, input: &str, tier: Tier) -> FuzzResult<ExecutionResult> {
        let start = Instant::now();

        // Placeholder execution - real implementation would use verum_compiler/interpreter
        // For now, we simulate execution
        let success = !input.contains("error") && !input.contains("FAIL");
        let output = if success {
            Some(format!("result_from_{:?}", tier))
        } else {
            None
        };

        Ok(ExecutionResult {
            success,
            output,
            error: if success {
                None
            } else {
                Some("Simulated error".to_string())
            },
            duration_ns: start.elapsed().as_nanos() as u64,
            memory_bytes: 0,
            tier,
        })
    }

    /// Check if an input is interesting (would increase coverage)
    fn is_interesting(&self, _input: &str) -> bool {
        // Placeholder - real implementation would track coverage
        // For now, use a simple heuristic
        self.rng.clone().random::<f64>() < 0.01 // 1% chance of being interesting
    }

    /// Handle a found issue
    fn handle_issue(&mut self, mut issue: Issue) -> FuzzResult<()> {
        match &issue.kind {
            IssueKind::Crash(_) => {
                self.stats.crashes += 1;

                // Check for uniqueness
                if !self.is_duplicate_crash(&issue) {
                    self.stats.unique_crashes += 1;
                }
            }
            IssueKind::DifferentialMismatch => {
                self.stats.differential_bugs += 1;
            }
            IssueKind::Timeout => {
                self.stats.timeouts += 1;
            }
            _ => {}
        }

        // Minimize if configured
        if self.config.minimize {
            if let Some(minimized) = self.minimize_issue(&issue)? {
                issue.minimized = Some(minimized);
            }
        }

        // Save the issue
        issue.save(&self.config.crash_dir)?;
        self.issues.push(issue);

        Ok(())
    }

    /// Minimize a failing input
    fn minimize_issue(&self, issue: &Issue) -> FuzzResult<Option<String>> {
        // Use a simple test function that checks for the same pattern
        let issue_kind = issue.kind.clone();
        let test_fn = move |input: &str| -> bool {
            // Simple heuristic: check if the input still contains
            // patterns that suggest the same issue
            match &issue_kind {
                IssueKind::Crash(CrashKind::Panic) => {
                    input.contains("panic!") || input.contains("unreachable!")
                }
                IssueKind::DifferentialMismatch => {
                    // For differential, just check if input is non-trivial
                    input.len() > 10
                }
                _ => !input.is_empty(),
            }
        };

        match self.shrinker.shrink(&issue.input, test_fn) {
            ShrinkResult::Success(minimized) => Ok(Some(minimized)),
            ShrinkResult::NoProgress => Ok(None),
            ShrinkResult::Error(e) => {
                if self.config.verbose {
                    eprintln!("Shrinking failed: {}", e);
                }
                Ok(None)
            }
        }
    }

    /// Check if a crash is a duplicate
    fn is_duplicate_crash(&self, issue: &Issue) -> bool {
        // Simple deduplication by message hash
        let hash = self.hash_input(&issue.message);
        self.issues.iter().any(|i| {
            if let IssueKind::Crash(_) = &i.kind {
                self.hash_input(&i.message) == hash
            } else {
                false
            }
        })
    }

    /// Hash an input for deduplication
    fn hash_input(&self, input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let hash = hasher.finalize();
        hex::encode(&hash[..16])
    }

    /// Stop the fuzzing campaign
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Get current statistics
    pub fn stats(&self) -> &FuzzStats {
        &self.stats
    }

    /// Get all found issues
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }

    /// Get the corpus manager
    pub fn corpus(&self) -> &CorpusManager {
        &self.corpus
    }
}

/// Get number of CPUs
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_fuzz_config_default() {
        let config = FuzzConfig::default();
        assert!(config.differential);
        assert!(config.minimize);
        assert!(config.workers > 0);
    }

    #[test]
    fn test_issue_creation() {
        let issue = Issue::new("fn main() {}", IssueKind::Crash(CrashKind::Panic), "test");
        assert!(!issue.id.is_empty());
        assert_eq!(issue.input, "fn main() {}");
    }

    #[test]
    fn test_issue_save() {
        let dir = tempdir().unwrap();
        let issue = Issue::new("fn main() {}", IssueKind::Crash(CrashKind::Panic), "test");
        let path = issue.save(dir.path()).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_fuzz_engine_creation() {
        let dir = tempdir().unwrap();
        let config = FuzzConfig {
            crash_dir: dir.path().join("crashes"),
            corpus_dir: dir.path().join("corpus"),
            ..Default::default()
        };
        let engine = FuzzEngine::new(config);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_fuzz_engine_run() {
        let dir = tempdir().unwrap();
        let config = FuzzConfig {
            crash_dir: dir.path().join("crashes"),
            corpus_dir: dir.path().join("corpus"),
            iterations: 10, // Small number for quick test
            verbose: false,
            minimize: false,     // Disable minimization for speed
            differential: false, // Disable differential for speed
            ..Default::default()
        };
        let mut engine = FuzzEngine::new(config).unwrap();
        let stats = engine.run(10).unwrap();
        assert_eq!(stats.iterations, 10);
    }
}
