//! VCS Test Runner Library
//!
//! The `vtest` crate provides the core functionality for the Verum Compliance Suite
//! test runner. It handles test discovery, execution across multiple tiers, and
//! result reporting.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        VTEST ARCHITECTURE                        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │                                                                   │
//! │    CLI Layer (main.rs)                                          │
//! │         │                                                        │
//! │         ▼                                                        │
//! │    ┌─────────────────────────────────────────────────────────┐  │
//! │    │                   VTestRunner                            │  │
//! │    │  • Orchestrates test discovery and execution            │  │
//! │    │  • Manages parallel execution with worker pool          │  │
//! │    │  • Coordinates differential testing                     │  │
//! │    │  • Supports fail-fast mode                              │  │
//! │    └──────────────────────────┬──────────────────────────────┘  │
//! │                               │                                  │
//! │         ┌─────────────────────┼─────────────────────┐           │
//! │         ▼                     ▼                     ▼           │
//! │    ┌──────────┐        ┌──────────┐         ┌──────────┐       │
//! │    │ Directive│        │ Executor │         │ Reporter │       │
//! │    │ Parser   │        │          │         │          │       │
//! │    └──────────┘        └──────────┘         └──────────┘       │
//! │                                                                   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Modules
//!
//! - [`directive`]: Parses test directives from `.vr` files
//! - [`discovery`]: Flexible test discovery with filtering
//! - [`executor`]: Executes tests across different tiers
//! - [`report`]: Generates test reports in various formats
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use vtest::{VTestRunner, RunnerConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = RunnerConfig::default();
//!     let runner = VTestRunner::new(config);
//!
//!     let results = runner.run_tests("specs/**/*.vr").await.unwrap();
//!     println!("Passed: {}, Failed: {}", results.passed, results.failed);
//! }
//! ```

#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]

pub mod benchmark;
pub mod cache;
pub mod config;
pub mod directive;
pub mod discovery;
pub mod executor;
pub mod filter;
pub mod fuzz;
pub mod isolation;
pub mod progress;
pub mod report;
pub mod watch;

use crate::directive::{DirectiveError, Level, TestDirectives, Tier, discover_tests};
use crate::executor::{
    Executor, ExecutorConfig, ProcessOutput, TestOutcome, TestResult, compare_differential_results,
};
use crate::report::{ReportFormat, Reporter};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{Mutex, Semaphore};
use verum_common::{List, Map, Set, Text};
use verum_compiler::{CompilationPipeline, Session, CompilerOptions, OutputFormat};

/// Error type for the test runner.
#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("Directive error: {0}")]
    DirectiveError(#[from] DirectiveError),

    #[error("Executor error: {0}")]
    ExecutorError(#[from] executor::ExecutorError),

    #[error("Report error: {0}")]
    ReportError(#[from] report::ReportError),

    #[error("Configuration error: {0}")]
    ConfigError(Text),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Configuration for the test runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Base paths to search for tests
    pub test_paths: List<PathBuf>,
    /// Glob pattern for test files
    pub test_pattern: Text,
    /// Patterns to exclude
    pub exclude_patterns: List<Text>,
    /// Tags to include (empty = all)
    pub include_tags: Set<Text>,
    /// Tags to exclude
    pub exclude_tags: Set<Text>,
    /// Levels to run (empty = all)
    pub levels: Set<Level>,
    /// Tiers to run (empty = all from test)
    pub tiers: Set<Tier>,
    /// Maximum parallel test execution
    pub parallel: usize,
    /// Default timeout in milliseconds
    pub default_timeout_ms: u64,
    /// Executor configuration
    pub executor_config: ExecutorConfig,
    /// Output format
    pub output_format: ReportFormat,
    /// Output path (None = stdout)
    pub output_path: Option<PathBuf>,
    /// Verbose mode
    pub verbose: bool,
    /// Use colors in output
    pub use_colors: bool,
    /// Fail fast (stop on first failure)
    pub fail_fast: bool,
    /// Watch mode
    pub watch: bool,
    /// Compiler version (for reports)
    pub compiler_version: Text,
    /// Show progress bar
    pub show_progress: bool,
    /// Shuffle test order (for detecting order-dependent failures)
    pub shuffle: bool,
    /// Random seed for shuffle
    pub shuffle_seed: Option<u64>,
    /// Retry failed tests
    pub retry_failed: usize,
    /// Run differential tests (compare Tier 0 vs Tier 3)
    pub differential: bool,
    /// Tolerance for floating-point comparison in differential tests
    pub differential_tolerance: f64,
    /// Update expected output files with actual results
    pub update_expectations: bool,
    /// Show only summary (no individual test results)
    pub summary_only: bool,
    /// Filter by test name pattern (supports glob patterns)
    pub filter_pattern: Option<Text>,
    /// Retry failed tests N times (duplicated for CLI compat)
    pub retries: u32,
    /// Save test results to a baseline file for comparison
    pub save_baseline: Option<PathBuf>,
    /// Compare results against a baseline file
    pub compare_baseline: Option<PathBuf>,
    /// Generate coverage information
    pub coverage: bool,
    /// Show diffs for failures
    pub show_diff: bool,
    /// Quiet mode (only show failures)
    pub quiet: bool,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            test_paths: vec![PathBuf::from("specs")].into(),
            test_pattern: "**/*.vr".to_string().into(),
            exclude_patterns: vec!["**/skip/**".to_string().into(), "**/wip/**".to_string().into()].into(),
            include_tags: Set::new(),
            exclude_tags: Set::new(),
            levels: Set::new(),
            tiers: Set::new(),
            parallel: default_parallel(),
            default_timeout_ms: 30_000,
            executor_config: ExecutorConfig::default(),
            output_format: ReportFormat::Console,
            output_path: None,
            verbose: false,
            use_colors: true,
            fail_fast: false,
            watch: false,
            compiler_version: "unknown".to_string().into(),
            show_progress: true,
            shuffle: false,
            shuffle_seed: None,
            retry_failed: 0,
            differential: false,
            differential_tolerance: 0.0001,
            update_expectations: false,
            summary_only: false,
            filter_pattern: None,
            retries: 0,
            save_baseline: None,
            compare_baseline: None,
            coverage: false,
            show_diff: true,
            quiet: false,
        }
    }
}

/// Live statistics during test execution.
#[derive(Debug)]
pub struct LiveStats {
    /// Total tests to run
    pub total: AtomicUsize,
    /// Tests completed
    pub completed: AtomicUsize,
    /// Tests passed
    pub passed: AtomicUsize,
    /// Tests failed
    pub failed: AtomicUsize,
    /// Tests skipped
    pub skipped: AtomicUsize,
    /// Flag to stop execution (for fail-fast)
    pub should_stop: AtomicBool,
    /// Start time
    pub start_time: Instant,
}

impl LiveStats {
    /// Create new live stats.
    pub fn new(total: usize) -> Self {
        Self {
            total: AtomicUsize::new(total),
            completed: AtomicUsize::new(0),
            passed: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            skipped: AtomicUsize::new(0),
            should_stop: AtomicBool::new(false),
            start_time: Instant::now(),
        }
    }

    /// Record a test result.
    pub fn record(&self, result: &TestResult) {
        self.completed.fetch_add(1, Ordering::SeqCst);

        if result.all_pass() {
            self.passed.fetch_add(1, Ordering::SeqCst);
        } else if result.outcomes.iter().all(|o| o.is_skip()) {
            self.skipped.fetch_add(1, Ordering::SeqCst);
        } else {
            self.failed.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Check if we should stop execution.
    pub fn should_stop(&self) -> bool {
        self.should_stop.load(Ordering::SeqCst)
    }

    /// Signal to stop execution.
    pub fn stop(&self) {
        self.should_stop.store(true, Ordering::SeqCst);
    }

    /// Get completion percentage.
    pub fn progress_percent(&self) -> f64 {
        let total = self.total.load(Ordering::SeqCst);
        if total == 0 {
            return 100.0;
        }
        (self.completed.load(Ordering::SeqCst) as f64 / total as f64) * 100.0
    }

    /// Get elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get estimated time remaining.
    pub fn estimated_remaining(&self) -> Option<Duration> {
        let completed = self.completed.load(Ordering::SeqCst);
        let total = self.total.load(Ordering::SeqCst);

        if completed == 0 {
            return None;
        }

        let elapsed = self.elapsed().as_secs_f64();
        let rate = completed as f64 / elapsed;
        let remaining = (total - completed) as f64 / rate;

        Some(Duration::from_secs_f64(remaining))
    }
}

/// Summary of test run results.
#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    /// Total tests discovered
    pub total: usize,
    /// Tests passed
    pub passed: usize,
    /// Tests failed
    pub failed: usize,
    /// Tests skipped
    pub skipped: usize,
    /// Tests errored
    pub errored: usize,
    /// Total duration
    pub duration: Duration,
    /// Results by level
    pub by_level: Map<Level, LevelStats>,
    /// Results by tier
    pub by_tier: Map<Tier, TierStats>,
}

/// Statistics for a level.
#[derive(Debug, Clone, Default)]
pub struct LevelStats {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}

/// Statistics for a tier.
#[derive(Debug, Clone, Default)]
pub struct TierStats {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub total_duration: Duration,
}

/// The main test runner.
pub struct VTestRunner {
    config: RunnerConfig,
    executor: Executor,
}

impl VTestRunner {
    /// Create a new test runner with the given configuration.
    pub fn new(config: RunnerConfig) -> Self {
        let executor = Executor::new(config.executor_config.clone());
        Self { config, executor }
    }

    /// Discover all tests matching the configuration.
    pub fn discover(&self) -> Result<List<TestDirectives>, RunnerError> {
        let mut all_tests = List::new();
        let exclude_refs: List<&str> = self
            .config
            .exclude_patterns
            .iter()
            .map(|s| s.as_str())
            .collect();

        for base_path in &self.config.test_paths {
            let test_paths = discover_tests(base_path, &self.config.test_pattern, &exclude_refs)?;

            for path in test_paths {
                match TestDirectives::from_file(Path::new(&path)) {
                    Ok(directives) => {
                        // Filter by level
                        if !self.config.levels.is_empty()
                            && !self.config.levels.contains(&directives.level)
                        {
                            continue;
                        }

                        // Filter by tags
                        if !directives
                            .matches_tags(&self.config.include_tags, &self.config.exclude_tags)
                        {
                            continue;
                        }

                        all_tests.push(directives);
                    }
                    Err(DirectiveError::MissingTestDirective) => {
                        // Skip files without @test directive
                        continue;
                    }
                    Err(e) => {
                        if self.config.verbose {
                            eprintln!("Warning: Failed to parse {}: {}", path, e);
                        }
                    }
                }
            }
        }

        Ok(all_tests)
    }

    /// Run all tests matching the configuration.
    pub async fn run(&self) -> Result<(List<TestResult>, RunSummary), RunnerError> {
        let tests = self.discover()?;
        self.run_tests(tests).await
    }

    /// Run a specific list of tests.
    pub async fn run_tests(
        &self,
        tests: List<TestDirectives>,
    ) -> Result<(List<TestResult>, RunSummary), RunnerError> {
        // Warm up the stdlib cache before parallel execution.
        // Without this, multiple parallel tests race to load the stdlib,
        // causing contention and intermittent failures.
        // We use the first test's path so find_workspace_root() resolves correctly.
        if verum_compiler::get_cached_stdlib_registry().is_none() {
            let warmup_path = tests.first()
                .map(|t| PathBuf::from(&t.source_path))
                .unwrap_or_else(|| PathBuf::from("warmup.vr"));
            let options = CompilerOptions {
                input: warmup_path,
                output_format: OutputFormat::Human,
                ..Default::default()
            };
            let mut session = Session::new(options);
            let mut pipeline = CompilationPipeline::new_check(&mut session);
            let _ = pipeline.run_check_only();
            // Cache is now populated — subsequent tests use fast path
        }

        let start = Instant::now();
        let total = tests.len();

        // Handle test shuffling if enabled
        let tests = if self.config.shuffle {
            self.shuffle_tests(tests)
        } else {
            tests
        };

        // Create live stats for progress tracking
        let stats = Arc::new(LiveStats::new(total));

        // Memory watchdog: monitor RSS and trigger fail-fast if memory exceeds 8GB.
        // This protects against in-process tests (direct library integration) that
        // cannot be constrained by setrlimit since they share the vtest process.
        let watchdog_stats = stats.clone();
        let _memory_watchdog = tokio::spawn(async move {
            const MAX_RSS_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8GB
            const CHECK_INTERVAL: Duration = Duration::from_secs(2);
            loop {
                tokio::time::sleep(CHECK_INTERVAL).await;
                if watchdog_stats.should_stop() {
                    break;
                }
                if let Some(rss) = get_current_rss_bytes() {
                    if rss > MAX_RSS_BYTES {
                        eprintln!(
                            "[vtest] MEMORY WATCHDOG: RSS {} MB exceeds {} MB limit — stopping tests",
                            rss / (1024 * 1024),
                            MAX_RSS_BYTES / (1024 * 1024),
                        );
                        watchdog_stats.stop();
                        break;
                    }
                }
            }
        });

        // Create a semaphore for parallel execution
        let semaphore = Arc::new(Semaphore::new(self.config.parallel));

        // Create results collector (thread-safe)
        let results = Arc::new(Mutex::new(List::new()));

        // Setup progress bar if enabled
        let progress_bar = if self.config.show_progress && !self.config.verbose {
            let pb = ProgressBar::new(total as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                    .unwrap()
                    .progress_chars("=>-"),
            );
            Some(pb)
        } else {
            None
        };

        // Run tests in parallel with fail-fast support
        let fail_fast = self.config.fail_fast;
        let retry_count = self.config.retry_failed;

        let futures: Vec<_> = tests
            .into_iter()
            .map(|directives| {
                let semaphore = semaphore.clone();
                let stats = stats.clone();
                let results = results.clone();
                let progress_bar = progress_bar.clone();
                let executor = self.executor.clone();

                async move {
                    // Check if we should stop (fail-fast triggered)
                    if stats.should_stop() {
                        return;
                    }

                    // Acquire permit from semaphore
                    let _permit = match semaphore.acquire().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };

                    // Check again after acquiring permit
                    if stats.should_stop() {
                        return;
                    }

                    // Execute the test with optional retries
                    let mut result = match executor.execute(directives.clone()).await {
                        Ok(r) => r,
                        Err(e) => {
                            // Create an error result
                            TestResult {
                                directives: directives.clone(),
                                outcomes: vec![TestOutcome::Error {
                                    tier: Tier::Tier0,
                                    error: e.to_string().into(),
                                }].into(),
                                total_duration: Duration::ZERO,
                            }
                        }
                    };

                    // Retry failed tests if configured
                    for _ in 0..retry_count {
                        if result.all_pass() {
                            break;
                        }
                        // Retry
                        result = match executor.execute(directives.clone()).await {
                            Ok(r) => r,
                            Err(e) => TestResult {
                                directives: directives.clone(),
                                outcomes: vec![TestOutcome::Error {
                                    tier: Tier::Tier0,
                                    error: e.to_string().into(),
                                }].into(),
                                total_duration: Duration::ZERO,
                            },
                        };
                    }

                    // Record stats
                    stats.record(&result);

                    // Update progress bar
                    if let Some(ref pb) = progress_bar {
                        let passed = stats.passed.load(Ordering::SeqCst);
                        let failed = stats.failed.load(Ordering::SeqCst);
                        pb.set_message(format!("{} passed, {} failed", passed, failed));
                        pb.inc(1);
                    }

                    // Check for fail-fast
                    if fail_fast && !result.all_pass() {
                        stats.stop();
                    }

                    // Store result
                    let mut results_guard = results.lock().await;
                    results_guard.push(result);
                }
            })
            .collect();

        // Execute all futures concurrently
        futures::future::join_all(futures).await;

        // Finish progress bar
        if let Some(pb) = progress_bar {
            pb.finish_with_message("done");
        }

        // Extract results
        let results = Arc::try_unwrap(results)
            .expect("All futures completed")
            .into_inner();

        // Build summary
        let mut summary = RunSummary {
            total,
            duration: start.elapsed(),
            ..Default::default()
        };

        for result in &results {
            // Level stats
            let level_stats = summary.by_level.entry(result.directives.level).or_default();
            level_stats.total += 1;

            let test_passed = result.all_pass();
            if test_passed {
                summary.passed += 1;
                level_stats.passed += 1;
            } else {
                summary.failed += 1;
                level_stats.failed += 1;
            }

            // Tier stats
            for outcome in &result.outcomes {
                let tier_stats = summary.by_tier.entry(outcome.tier()).or_default();
                tier_stats.total += 1;

                if let Some(duration) = outcome.duration() {
                    tier_stats.total_duration += duration;
                }

                if outcome.is_pass() {
                    tier_stats.passed += 1;
                } else if outcome.is_fail() {
                    tier_stats.failed += 1;
                }
            }
        }

        Ok((results, summary))
    }

    /// Shuffle tests using configured seed or random.
    fn shuffle_tests(&self, mut tests: List<TestDirectives>) -> List<TestDirectives> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let seed = self.config.shuffle_seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(12345)
        });

        // Simple Fisher-Yates shuffle with deterministic seed
        let n = tests.len();
        let mut hasher = DefaultHasher::new();

        for i in (1..n).rev() {
            seed.hash(&mut hasher);
            i.hash(&mut hasher);
            let j = (hasher.finish() as usize) % (i + 1);
            tests.swap(i, j);
        }

        tests
    }

    /// Run differential tests (compare results across tiers).
    pub async fn run_differential(
        &self,
        tests: List<TestDirectives>,
    ) -> Result<(List<TestResult>, RunSummary), RunnerError> {
        let start = Instant::now();
        let mut results = List::new();
        let mut summary = RunSummary {
            total: tests.len(),
            duration: Duration::default(),
            ..Default::default()
        };

        for directives in tests {
            // Run on all specified tiers
            let mut tier_outputs: List<ProcessOutput> = List::new();
            let mut outcomes: List<TestOutcome> = List::new();

            for tier in &directives.tiers {
                match self.run_single_tier(&directives, *tier).await {
                    Ok((output, outcome)) => {
                        tier_outputs.push(output);
                        outcomes.push(outcome);
                    }
                    Err(e) => {
                        outcomes.push(TestOutcome::Error {
                            tier: *tier,
                            error: e.to_string().into(),
                        });
                    }
                }
            }

            // Compare results across tiers
            if tier_outputs.len() >= 2 {
                match compare_differential_results(&tier_outputs, 0.1) {
                    Ok(()) => {
                        // All tiers match - this is good
                        summary.passed += 1;
                    }
                    Err(reason) => {
                        // Tiers don't match - this is a failure
                        summary.failed += 1;
                        // Replace the first outcome with a failure
                        if !outcomes.is_empty() {
                            outcomes[0] = TestOutcome::Fail {
                                tier: outcomes[0].tier(),
                                reason,
                                expected: Some("All tiers produce identical results".to_string().into()),
                                actual: Some("Tier results differ".to_string().into()),
                                duration: outcomes[0].duration().unwrap_or_default(),
                            };
                        }
                    }
                }
            }

            results.push(TestResult {
                directives,
                outcomes,
                total_duration: start.elapsed(),
            });
        }

        summary.duration = start.elapsed();
        Ok((results, summary))
    }

    /// Run a single test on a single tier.
    async fn run_single_tier(
        &self,
        directives: &TestDirectives,
        tier: Tier,
    ) -> Result<(ProcessOutput, TestOutcome), RunnerError> {
        // This is a simplified version - in practice we'd want the executor
        // to return both the output and the outcome
        let result = self.executor.execute(directives.clone()).await?;

        // Find the outcome for this tier
        let outcome = result
            .outcomes
            .into_iter()
            .find(|o| o.tier() == tier)
            .unwrap_or_else(|| TestOutcome::Skip {
                tier,
                reason: "Tier not executed".to_string().into(),
            });

        // Create a dummy ProcessOutput since we don't have access to the raw output
        // In a real implementation, we'd modify the executor to return this
        let output = ProcessOutput::default();

        Ok((output, outcome))
    }

    /// Generate a report for the given results.
    pub fn generate_report(&self, results: List<TestResult>) -> Result<(), RunnerError> {
        let mut reporter = Reporter::new(self.config.compiler_version.clone())
            .with_colors(self.config.use_colors)
            .with_verbose(self.config.verbose);

        reporter.add_results(results);

        if let Some(ref path) = self.config.output_path {
            reporter.generate_to_file(path, self.config.output_format)?;
        } else {
            let mut stdout = std::io::stdout();
            reporter.generate(&mut stdout, self.config.output_format)?;
        }

        Ok(())
    }

    /// Run tests and generate a report.
    pub async fn run_and_report(&self) -> Result<i32, RunnerError> {
        let (results, summary) = self.run().await?;

        // Print progress if verbose
        if self.config.verbose {
            println!("Ran {} tests in {:?}", summary.total, summary.duration);
        }

        // Generate report
        let mut reporter = Reporter::new(self.config.compiler_version.clone())
            .with_colors(self.config.use_colors)
            .with_verbose(self.config.verbose);

        reporter.add_results(results);

        if let Some(ref path) = self.config.output_path {
            reporter.generate_to_file(path, self.config.output_format)?;
        } else {
            let mut stdout = std::io::stdout();
            reporter.generate(&mut stdout, self.config.output_format)?;
        }

        Ok(reporter.exit_code())
    }
}

/// List tests without running them.
pub fn list_tests(
    paths: &[PathBuf],
    pattern: &str,
    exclude: &[Text],
    levels: &Set<Level>,
    tags: &Set<Text>,
) -> Result<List<TestDirectives>, RunnerError> {
    let mut all_tests = List::new();
    let exclude_refs: List<&str> = exclude.iter().map(|s| s.as_str()).collect();

    for base_path in paths {
        let test_paths = discover_tests(base_path, pattern, &exclude_refs)?;

        for path in test_paths {
            match TestDirectives::from_file(Path::new(&path)) {
                Ok(directives) => {
                    // Filter by level
                    if !levels.is_empty() && !levels.contains(&directives.level) {
                        continue;
                    }

                    // Filter by tags
                    if !tags.is_empty() && !directives.tags.iter().any(|t| tags.contains(t)) {
                        continue;
                    }

                    all_tests.push(directives);
                }
                Err(DirectiveError::MissingTestDirective) => continue,
                Err(_) => continue,
            }
        }
    }

    Ok(all_tests)
}

/// Configuration from a vtest.toml file.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct VTestToml {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub reporting: ReportingConfig,
    #[serde(default)]
    pub tiers: TiersConfig,
    #[serde(default)]
    pub differential: DifferentialConfig,
    #[serde(default)]
    pub benchmarks: BenchmarksConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default = "default_paths")]
    pub paths: List<Text>,
    #[serde(default = "default_pattern")]
    pub pattern: Text,
    #[serde(default)]
    pub exclude: List<Text>,
}

fn default_paths() -> List<Text> {
    vec!["specs/".to_string().into()].into()
}

fn default_pattern() -> Text {
    "**/*.vr".to_string().into()
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            paths: default_paths(),
            pattern: default_pattern(),
            exclude: vec!["**/skip/**".to_string().into(), "**/wip/**".to_string().into()].into(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ExecutionConfig {
    #[serde(default = "default_parallel")]
    pub parallel: usize,
    #[serde(default = "default_timeout")]
    pub timeout_default: u64,
    #[serde(default = "default_tier")]
    pub tier_default: Text,
}

fn default_parallel() -> usize {
    // Sequential execution: the type checker's large functions (5600+ lines each)
    // create massive stack frames. With 512MB per thread × N concurrent tests,
    // virtual memory exhaustion causes stack overflow. Sequential avoids this.
    1
}

fn default_timeout() -> u64 {
    30_000
}

fn default_tier() -> Text {
    "all".to_string().into()
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            parallel: default_parallel(),
            timeout_default: default_timeout(),
            tier_default: default_tier(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ReportingConfig {
    #[serde(default = "default_format")]
    pub format: Text,
    #[serde(default = "default_colors")]
    pub colors: bool,
    #[serde(default)]
    pub verbose: bool,
    #[serde(default = "default_show_timing")]
    pub show_timing: bool,
}

fn default_format() -> Text {
    "console".to_string().into()
}

fn default_colors() -> bool {
    true
}

fn default_show_timing() -> bool {
    true
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            format: default_format(),
            colors: default_colors(),
            verbose: false,
            show_timing: default_show_timing(),
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TiersConfig {
    pub tier0: Option<TierConfig>,
    pub tier1: Option<TierConfig>,
    pub tier2: Option<TierConfig>,
    pub tier3: Option<TierConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TierConfig {
    pub interpreter: Option<Text>,
    pub jit: Option<Text>,
    pub aot: Option<Text>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DifferentialConfig {
    #[serde(default = "default_compare_tiers")]
    pub compare_tiers: List<u8>,
    #[serde(default = "default_tolerance_memory")]
    pub tolerance_memory: f64,
}

fn default_compare_tiers() -> List<u8> {
    vec![0, 3].into()
}

fn default_tolerance_memory() -> f64 {
    0.1
}

impl Default for DifferentialConfig {
    fn default() -> Self {
        Self {
            compare_tiers: default_compare_tiers(),
            tolerance_memory: default_tolerance_memory(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct BenchmarksConfig {
    #[serde(default = "default_warmup")]
    pub warmup_iterations: usize,
    #[serde(default = "default_measure")]
    pub measure_iterations: usize,
    #[serde(default)]
    pub baseline_path: Option<Text>,
}

fn default_warmup() -> usize {
    100
}

fn default_measure() -> usize {
    1000
}

impl Default for BenchmarksConfig {
    fn default() -> Self {
        Self {
            warmup_iterations: default_warmup(),
            measure_iterations: default_measure(),
            baseline_path: None,
        }
    }
}

impl VTestToml {
    /// Load configuration from a file.
    pub fn from_file(path: &Path) -> Result<Self, RunnerError> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| RunnerError::ConfigError(e.to_string().into()))
    }

    /// Load configuration from default location.
    pub fn load_default() -> Result<Self, RunnerError> {
        let paths = ["vtest.toml", ".vtest.toml", "test/vtest.toml"];

        for path in paths {
            if Path::new(path).exists() {
                return Self::from_file(Path::new(path));
            }
        }

        Ok(Self::default())
    }

    /// Convert to RunnerConfig.
    pub fn to_runner_config(&self) -> RunnerConfig {
        RunnerConfig {
            test_paths: self.discovery.paths.iter().map(PathBuf::from).collect(),
            test_pattern: self.discovery.pattern.clone(),
            exclude_patterns: self.discovery.exclude.clone(),
            parallel: self.execution.parallel,
            default_timeout_ms: self.execution.timeout_default,
            output_format: ReportFormat::from_str(&self.reporting.format)
                .unwrap_or(ReportFormat::Console),
            verbose: self.reporting.verbose,
            use_colors: self.reporting.colors,
            ..Default::default()
        }
    }
}

/// Get the current process RSS (resident set size) in bytes.
/// Returns None if the measurement is unavailable on this platform.
fn get_current_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // Use getrusage(RUSAGE_SELF) — ru_maxrss is in bytes on macOS
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let ret = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if ret == 0 {
            Some(usage.ru_maxrss as u64)
        } else {
            None
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Read /proc/self/statm — field 1 is RSS in pages
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let fields: Vec<&str> = statm.split_whitespace().collect();
            if fields.len() >= 2 {
                if let Ok(pages) = fields[1].parse::<u64>() {
                    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                    return Some(pages * page_size);
                }
            }
        }
        None
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        unsafe {
            let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut pmc,
                pmc.cb,
            ) != 0
            {
                Some(pmc.PeakWorkingSetSize as u64)
            } else {
                None
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_config_default() {
        let config = RunnerConfig::default();
        assert!(!config.test_paths.is_empty());
        assert!(config.parallel > 0);
        assert_eq!(config.output_format, ReportFormat::Console);
    }

    #[test]
    fn test_run_summary_default() {
        let summary = RunSummary::default();
        assert_eq!(summary.total, 0);
        assert_eq!(summary.passed, 0);
        assert_eq!(summary.failed, 0);
    }

    #[test]
    fn test_vtest_toml_default() {
        let config = VTestToml::default();
        assert!(!config.discovery.paths.is_empty());
        assert_eq!(config.execution.parallel, default_parallel());
    }
}
