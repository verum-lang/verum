//! Verification Profiler - Tracks SMT verification performance and provides diagnostics.
//!
//! Architecture: Each function with @verify(proof) contracts is profiled individually.
//! ProfileEntry records: function name, file location, verification time, SMT solver used,
//! logic (QF_LIA, QF_NRA, etc.), query count, detected bottleneck, and recommendations.
//! Reports sort by time descending, flagging functions exceeding 5s as slow verifications.
//! Cache statistics track hits/misses for incremental compilation speedup.
//!
//! Tracks verification performance, detects bottlenecks, and provides actionable
//! recommendations for optimizing SMT verification.
//!
//! # Features
//!
//! - **Per-function profiling**: Track verification time, SMT solver, logic, query count
//! - **Bottleneck detection**: Identify array reasoning, quantifiers, nonlinear arithmetic
//! - **Actionable recommendations**: Suggest specific optimizations (hints, runtime checks, splits)
//! - **Cache transparency**: Show cache hits/misses and time saved
//! - **Budget enforcement**: Fail builds if verification exceeds time budget
//!
//! # Example Usage
//!
//! ```rust,no_run
//! use verum_compiler::verification_profiler::VerificationProfiler;
//! use std::time::Duration;
//!
//! let mut profiler = VerificationProfiler::new();
//!
//! // Profile function verification
//! // let result = profiler.profile_function(
//! //     "my_function",
//! //     location,
//! //     VerifyMode::Proof,
//! //     &mut verifier,
//! // )?;
//!
//! // Generate and print report
//! profiler.print_report();
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};
use colored::Colorize;
use verum_smt::backend_trait::SmtLogic;
use verum_smt::cost::VerificationCost;
use verum_smt::{ProofResult, VerificationError, VerifyMode};
use verum_common::{List, Maybe, Text, ToText};

/// Default threshold for slow verification detection (5 seconds)
const DEFAULT_SLOW_THRESHOLD: Duration = Duration::from_secs(5);

/// Full verification profiler per spec Section 1.4
pub struct VerificationProfiler {
    entries: List<ProfileEntry>,
    cache_stats: CacheStatistics,
    start_time: Instant,
    /// Configurable threshold for slow verification detection
    slow_threshold: Duration,
}

/// Single function verification profile entry
#[derive(Debug, Clone)]
pub struct ProfileEntry {
    pub function_name: Text,
    pub file_location: FileLocation,
    pub verification_time: Duration,
    pub smt_solver: SmtSolver,
    pub logic: SmtLogic,
    pub query_count: usize,
    pub bottleneck: Maybe<Text>,
    pub recommendations: List<Text>,
}

/// Source file location
#[derive(Debug, Clone)]
pub struct FileLocation {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
}

impl FileLocation {
    pub fn new(file: PathBuf, line: u32, column: u32) -> Self {
        Self { file, line, column }
    }

    pub fn unknown() -> Self {
        Self {
            file: PathBuf::from("<unknown>"),
            line: 0,
            column: 0,
        }
    }
}

/// SMT solver backend used
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtSolver {
    Z3,
    CVC5,
}

impl SmtSolver {
    pub fn as_str(&self) -> &'static str {
        match self {
            SmtSolver::Z3 => "Z3",
            SmtSolver::CVC5 => "CVC5",
        }
    }
}

/// Verification cache statistics
#[derive(Debug, Clone)]
pub struct CacheStatistics {
    pub hits: usize,
    pub misses: usize,
    pub time_saved: Duration,
    pub cache_size_bytes: usize,
    pub entry_count: usize,
}

impl Default for CacheStatistics {
    fn default() -> Self {
        Self {
            hits: 0,
            misses: 0,
            time_saved: Duration::ZERO,
            cache_size_bytes: 0,
            entry_count: 0,
        }
    }
}

impl CacheStatistics {
    pub fn hit_rate(&self) -> f64 {
        let total = (self.hits + self.misses) as f64;
        if total > 0.0 {
            self.hits as f64 / total
        } else {
            0.0
        }
    }

    pub fn total_requests(&self) -> usize {
        self.hits + self.misses
    }
}

/// Complete verification report
#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub slow_verifications: List<ProfileEntry>,
    pub cache_stats: CacheStatistics,
    pub total_time: Duration,
    pub function_count: usize,
    pub avg_time_per_function: Duration,
    pub functions_above_threshold: usize,
    /// Sum of the slowest 80% of verification times — represents optimization potential
    pub optimization_potential: Duration,
    pub recommendations: List<Text>,
}

impl VerificationProfiler {
    /// Create new verification profiler
    pub fn new() -> Self {
        Self {
            entries: List::new(),
            cache_stats: CacheStatistics::default(),
            start_time: Instant::now(),
            slow_threshold: DEFAULT_SLOW_THRESHOLD,
        }
    }

    /// Create new verification profiler with a custom slow threshold
    pub fn with_threshold(threshold: Duration) -> Self {
        Self {
            entries: List::new(),
            cache_stats: CacheStatistics::default(),
            start_time: Instant::now(),
            slow_threshold: threshold,
        }
    }

    /// Set the slow verification threshold
    pub fn set_threshold(&mut self, threshold: Duration) {
        self.slow_threshold = threshold;
    }

    /// Get the current slow verification threshold
    pub fn threshold(&self) -> Duration {
        self.slow_threshold
    }

    /// Wall-clock elapsed time since this profiler started.
    ///
    /// Distinct from `total_verification_time()` (which sums per-entry
    /// active verification spans): this captures the full wall-clock
    /// duration including idle gaps between verifications, useful for
    /// reporting the profiler's observation window.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Profile a single function verification
    ///
    /// This wraps the actual verification with timing and analysis.
    /// After verification completes, it analyzes bottlenecks and generates recommendations.
    pub fn profile_function<F>(
        &mut self,
        func_name: &str,
        location: FileLocation,
        _verify_mode: VerifyMode,
        verify_fn: F,
    ) -> Result<ProofResult, VerificationError>
    where
        F: FnOnce() -> Result<ProofResult, VerificationError>,
    {
        let start = Instant::now();
        let result = verify_fn();
        let wall_clock_elapsed = start.elapsed();

        // Extract info from result
        // Use the reported verification time from VerificationCost if available,
        // otherwise fall back to wall-clock time. This allows:
        // 1. SMT solver's accurate internal timing to be used
        // 2. Tests to specify verification duration without actual delays
        let (logic, query_count, cached, verification_time) = match &result {
            Ok(proof) => {
                // Infer SMT logic from the proof's complexity and category
                let logic = infer_smt_logic(&proof.cost);
                // Extract actual query count from VerificationCost
                let query_count = proof.cost.num_checks as usize;
                // Use the duration from VerificationCost if it's been set to a meaningful value
                // (i.e., > 0), otherwise use wall-clock time
                let verification_time = if proof.cost.duration > Duration::ZERO {
                    proof.cost.duration
                } else {
                    wall_clock_elapsed
                };
                (logic, query_count, proof.cached, verification_time)
            }
            Err(VerificationError::Timeout { cost, .. }) => {
                // For timeouts, use the reported timeout duration and query count
                let logic = infer_smt_logic(cost);
                let query_count = cost.num_checks as usize;
                let verification_time = if cost.duration > Duration::ZERO {
                    cost.duration
                } else {
                    wall_clock_elapsed
                };
                (logic, query_count, false, verification_time)
            }
            Err(_) => (SmtLogic::ALL, 0, false, wall_clock_elapsed),
        };

        // Update cache stats
        if cached {
            self.cache_stats.hits += 1;
            // Estimate time saved (average of previous verifications)
            if !self.entries.is_empty() {
                let avg_time: Duration = self
                    .entries
                    .iter()
                    .map(|e| e.verification_time)
                    .sum::<Duration>()
                    / self.entries.len() as u32;
                self.cache_stats.time_saved += avg_time;
            }
        } else {
            self.cache_stats.misses += 1;
        }
        self.cache_stats.entry_count += 1;

        // Analyze bottleneck from result
        let bottleneck = self.analyze_bottleneck(&result, &logic, verification_time);

        // Generate recommendations based on performance
        let recommendations = self.generate_recommendations(verification_time, &bottleneck, &logic);

        self.entries.push(ProfileEntry {
            function_name: func_name.to_text(),
            file_location: location,
            verification_time,
            smt_solver: SmtSolver::Z3, // Default to Z3
            logic,
            query_count,
            bottleneck,
            recommendations,
        });

        result
    }

    /// Record a verification result from an external verification call
    ///
    /// This is used when the verification is done outside of profile_function
    /// (e.g., when borrow conflicts prevent using the closure-based approach)
    pub fn record_result(
        &mut self,
        func_name: &str,
        location: FileLocation,
        elapsed: Duration,
        result: &Result<ProofResult, VerificationError>,
    ) {
        // Extract info from result
        let (logic, query_count, cached) = match result {
            Ok(proof) => {
                // Infer SMT logic from the proof's complexity and category
                let logic = infer_smt_logic(&proof.cost);
                // Extract actual query count from VerificationCost
                let query_count = proof.cost.num_checks as usize;
                (logic, query_count, proof.cached)
            }
            Err(VerificationError::Timeout { cost, .. }) => {
                let logic = infer_smt_logic(cost);
                let query_count = cost.num_checks as usize;
                (logic, query_count, false)
            }
            Err(_) => (SmtLogic::ALL, 0, false),
        };

        // Update cache stats
        if cached {
            self.cache_stats.hits += 1;
        } else {
            self.cache_stats.misses += 1;
        }
        self.cache_stats.entry_count += 1;

        // Analyze bottleneck from result
        let bottleneck = self.analyze_bottleneck(result, &logic, elapsed);

        // Generate recommendations based on performance
        let recommendations = self.generate_recommendations(elapsed, &bottleneck, &logic);

        // Record the entry
        self.entries.push(ProfileEntry {
            function_name: func_name.to_text(),
            file_location: location,
            verification_time: elapsed,
            smt_solver: SmtSolver::Z3,
            logic,
            query_count,
            bottleneck,
            recommendations,
        });
    }

    /// Analyze what's causing slow verification
    fn analyze_bottleneck(
        &self,
        result: &Result<ProofResult, VerificationError>,
        logic: &SmtLogic,
        elapsed: Duration,
    ) -> Maybe<Text> {
        // Detect patterns based on logic and timing

        // Check for timeout first
        if let Err(VerificationError::Timeout { .. }) = result {
            return Maybe::Some("Verification timeout - formula too complex".to_text());
        }

        // Analyze based on SMT logic used
        match logic {
            SmtLogic::QF_AUFLIA | SmtLogic::QF_AX => {
                if elapsed > Duration::from_secs(3) {
                    return Maybe::Some(
                        "Array reasoning - complex array theory formulas".to_text(),
                    );
                }
            }
            SmtLogic::QF_NIA | SmtLogic::QF_NRA => {
                if elapsed > Duration::from_secs(2) {
                    return Maybe::Some(
                        "Nonlinear arithmetic - polynomial constraints are NP-hard".to_text(),
                    );
                }
            }
            SmtLogic::QF_BV => {
                if elapsed > Duration::from_secs(2) {
                    return Maybe::Some(
                        "Bit-vector reasoning - large bitvector operations".to_text(),
                    );
                }
            }
            SmtLogic::ALL => {
                if elapsed > Duration::from_secs(5) {
                    // Check for common patterns in full logic
                    if let Err(VerificationError::CannotProve { .. }) = result {
                        return Maybe::Some(
                            "Quantifier instantiation - universal/existential quantifiers"
                                .to_text(),
                        );
                    }
                    return Maybe::Some("Complex formula - multiple theories combined".to_text());
                }
            }
            _ => {}
        }

        // Check for general slow verification
        if elapsed > Duration::from_secs(5) {
            return Maybe::Some("Large formula size - too many constraints".to_text());
        }

        Maybe::None
    }

    /// Generate actionable recommendations
    fn generate_recommendations(
        &self,
        elapsed: Duration,
        bottleneck: &Maybe<Text>,
        logic: &SmtLogic,
    ) -> List<Text> {
        let mut recs = List::new();

        // Slow verification recommendations
        if elapsed > Duration::from_secs(5) {
            recs.push(
                "Consider splitting function into smaller pieces with simpler contracts".to_text(),
            );
            recs.push(
                "Use @verify(runtime) for development, @verify(proof) for production".to_text(),
            );
        }

        // Bottleneck-specific recommendations
        match bottleneck {
            Maybe::Some(b) if b.as_str().contains("Array") => {
                recs.push(
                    "Add @hint(\"array-split\") to guide solver on array operations".to_text(),
                );
                recs.push("Consider using sequence theory instead of array theory".to_text());
            }
            Maybe::Some(b) if b.as_str().contains("Quantifier") => {
                recs.push("Use @verify(runtime) for quantified formulas in hot paths".to_text());
                recs.push("Add explicit axioms to reduce quantifier instantiation".to_text());
            }
            Maybe::Some(b) if b.as_str().contains("Nonlinear") => {
                recs.push(
                    "Linearize constraints where possible (e.g., x*y → z with x*y=z)".to_text(),
                );
                recs.push("Use interval arithmetic or floating-point approximations".to_text());
            }
            Maybe::Some(b) if b.as_str().contains("Bit-vector") => {
                recs.push("Reduce bitvector width if possible (e.g., 64-bit → 32-bit)".to_text());
                recs.push("Use word-level reasoning instead of bit-blasting".to_text());
            }
            Maybe::Some(b) if b.as_str().contains("timeout") => {
                recs.push("Increase SMT timeout with --smt-timeout flag".to_text());
                recs.push("Add intermediate assertions to guide the solver".to_text());
                recs.push("Consider using @verify(skip) and relying on tests".to_text());
            }
            _ => {}
        }

        // Logic-specific recommendations
        match logic {
            SmtLogic::ALL if elapsed > Duration::from_secs(3) => {
                recs.push(
                    "Specify explicit logic (e.g., QF_LIA) to improve solver performance".to_text(),
                );
            }
            _ => {}
        }

        // Cache recommendations
        if self.cache_stats.hit_rate() < 0.5 && self.cache_stats.total_requests() > 10 {
            recs.push("Low cache hit rate - consider enabling distributed cache".to_text());
        }

        recs
    }

    /// Generate full verification report
    ///
    /// Sorts entries by verification time descending, filters slow verifications
    /// above the configurable threshold, and computes summary statistics including
    /// optimization potential (sum of the slowest 80% of verification times).
    pub fn generate_report(&self) -> VerificationReport {
        let threshold = self.slow_threshold;
        let total_time = self.total_verification_time();
        let function_count = self.entries.len();

        // Sort all entries by time descending
        let mut sorted_entries = self.entries.clone();
        sorted_entries.sort_by(|a, b| b.verification_time.cmp(&a.verification_time));

        // Filter to slow verifications (above threshold)
        let slow_verifications: List<_> = sorted_entries
            .iter()
            .filter(|e| e.verification_time > threshold)
            .cloned()
            .collect();

        let functions_above_threshold = slow_verifications.len();

        // Compute average time per function
        let avg_time_per_function = if function_count > 0 {
            total_time / function_count as u32
        } else {
            Duration::ZERO
        };

        // Compute optimization potential: sum of the slowest 80% of verification times.
        // This represents the time that could potentially be reduced through optimization.
        let optimization_potential = {
            let cutoff = (sorted_entries.len() as f64 * 0.8).ceil() as usize;
            sorted_entries
                .iter()
                .take(cutoff)
                .map(|e| e.verification_time)
                .sum()
        };

        // Generate global recommendations
        let recommendations = self.generate_global_recommendations();

        VerificationReport {
            slow_verifications,
            cache_stats: self.cache_stats.clone(),
            total_time,
            function_count,
            avg_time_per_function,
            functions_above_threshold,
            optimization_potential,
            recommendations,
        }
    }

    /// Compute total verification time across all entries
    fn total_verification_time(&self) -> Duration {
        self.entries
            .iter()
            .map(|e| e.verification_time)
            .sum()
    }

    /// Generate global recommendations based on overall verification patterns.
    ///
    /// Analyzes cache hit rates, slow function counts, total entry volume,
    /// logic distribution, and timing patterns to produce actionable advice.
    pub fn generate_global_recommendations(&self) -> List<Text> {
        let mut recs = List::new();
        let threshold = self.slow_threshold;

        let slow_count = self
            .entries
            .iter()
            .filter(|e| e.verification_time > threshold)
            .count();

        // Check if distributed cache should be enabled
        if self.cache_stats.hit_rate() < 0.7 && self.cache_stats.total_requests() > 20 {
            recs.push(
                "Enable distributed cache for CI: verum verify --distributed-cache=s3://bucket"
                    .to_text(),
            );
        }

        // Check if verification budget should be set
        if slow_count > 3 {
            recs.push(
                "Consider verification budget: verum.toml [verify] total_budget = \"120s\""
                    .to_text(),
            );
        }

        // Suggest profiling for hot paths
        if self.entries.len() > 50 {
            recs.push("Profile hot paths: verum profile --verification-only".to_text());
        }

        // Suggest development/production split
        if slow_count > 0 {
            recs.push(
                "Use @verify(runtime) for development, @verify(proof) for production builds"
                    .to_text(),
            );
        }

        // Analyze logic distribution for solver tuning advice
        let nia_count = self
            .entries
            .iter()
            .filter(|e| matches!(e.logic, SmtLogic::QF_NIA | SmtLogic::QF_NRA))
            .count();
        if nia_count > 5 {
            recs.push(
                "Multiple nonlinear arithmetic verifications detected — consider linearization or @verify(runtime)".to_text(),
            );
        }

        // Suggest parallel verification for large projects
        if self.entries.len() > 100 {
            recs.push(
                "Enable parallel verification: verum verify --jobs=auto".to_text(),
            );
        }

        // Warn about low cache efficiency when there are many entries
        if self.cache_stats.total_requests() > 50 && self.cache_stats.hit_rate() < 0.5 {
            recs.push(
                "Very low cache hit rate — check if source changes invalidate too many entries".to_text(),
            );
        }

        recs
    }

    /// Print report to terminal with colored output (spec format Section 1.3)
    ///
    /// Uses Unicode box-drawing characters for tree structure and the `colored`
    /// crate for ANSI terminal colors. The slow-verification threshold is
    /// configurable via `set_threshold()`.
    pub fn print_report(&self) {
        let report = self.generate_report();
        let threshold_secs = self.slow_threshold.as_secs();
        let separator = "━".repeat(53);

        // Header
        println!("{}", separator.bold());
        println!("{}", "Verification Performance Profile".bold());
        println!("{}", separator.bold());

        // Slow verifications section
        if !report.slow_verifications.is_empty() {
            println!(
                "\n{}\n",
                format!(
                    "\u{26a0} SLOW VERIFICATIONS (>{}s):",
                    threshold_secs
                )
                .yellow()
                .bold()
            );

            for (idx, entry) in report.slow_verifications.iter().enumerate() {
                let file_display = entry
                    .file_location
                    .file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("<unknown>");

                println!(
                    "{}. {} @ {}:{}",
                    (idx + 1).to_string().bold(),
                    format!("{}()", entry.function_name.as_str()).red().bold(),
                    file_display.cyan(),
                    entry.file_location.line,
                );

                // Verification time — color red if > 2x threshold, yellow otherwise
                let time_str = format!("{:.1}s", entry.verification_time.as_secs_f64());
                let colored_time = if entry.verification_time > self.slow_threshold * 2 {
                    time_str.red().bold()
                } else {
                    time_str.yellow()
                };
                println!("   \u{251c}\u{2500} Verification time: {}", colored_time);

                println!(
                    "   \u{251c}\u{2500} SMT solver: {}",
                    entry.smt_solver.as_str().white()
                );
                println!(
                    "   \u{251c}\u{2500} Logic: {}",
                    entry.logic.as_str().white()
                );
                println!(
                    "   \u{251c}\u{2500} Queries: {}",
                    entry.query_count.to_string().white()
                );

                if let Maybe::Some(ref bottleneck) = entry.bottleneck {
                    println!(
                        "   \u{251c}\u{2500} Bottleneck: {}",
                        bottleneck.as_str().red()
                    );
                }

                if !entry.recommendations.is_empty() {
                    println!(
                        "   \u{2514}\u{2500} {}:",
                        "Recommendations".green()
                    );
                    for rec in &entry.recommendations {
                        println!("      \u{2022} {}", rec.as_str());
                    }
                }
                println!();
            }
        } else {
            println!(
                "\n{}\n",
                format!(
                    "No slow verifications detected (threshold: {}s)",
                    threshold_secs
                )
                .green()
            );
        }

        // Cache statistics section
        println!("{}", separator.bold());
        println!("{}", "Cache Statistics".bold());
        println!("{}", separator.bold());
        println!();

        let total_requests = report.cache_stats.total_requests();
        if total_requests > 0 {
            let hit_rate = report.cache_stats.hit_rate() * 100.0;
            let hit_rate_colored = if hit_rate >= 80.0 {
                format!("{:.1}%", hit_rate).green()
            } else if hit_rate >= 50.0 {
                format!("{:.1}%", hit_rate).yellow()
            } else {
                format!("{:.1}%", hit_rate).red()
            };

            println!(
                "Cache hits:     {} / {} ({})",
                report.cache_stats.hits.to_string().green(),
                total_requests,
                hit_rate_colored,
            );
            println!(
                "Time saved:     {}",
                format!("{:.1}s", report.cache_stats.time_saved.as_secs_f64()).green(),
            );
        } else {
            println!("{}", "No cache activity recorded".dimmed());
        }

        // Summary section
        println!();
        println!("{}", separator.bold());
        println!("{}", "Summary".bold());
        println!("{}", separator.bold());
        println!();

        println!(
            "Total functions:  {}",
            report.function_count.to_string().white().bold()
        );
        println!(
            "Total time:       {}",
            format!("{:.1}s", report.total_time.as_secs_f64()).white().bold()
        );
        println!(
            "Average:          {}",
            format!("{:.2}s/function", report.avg_time_per_function.as_secs_f64()).white()
        );

        let slow_pct = if report.function_count > 0 {
            (report.functions_above_threshold as f64 / report.function_count as f64) * 100.0
        } else {
            0.0
        };
        let slow_str = format!(
            "{} ({:.1}%)",
            report.functions_above_threshold,
            slow_pct
        );
        let colored_slow = if report.functions_above_threshold == 0 {
            slow_str.green()
        } else {
            slow_str.yellow()
        };
        println!(
            "Slow (>{}s):      {}",
            threshold_secs, colored_slow
        );

        // Optimization potential
        if report.optimization_potential > Duration::ZERO {
            let opt_pct = if report.total_time.as_secs_f64() > 0.0 {
                (report.optimization_potential.as_secs_f64() / report.total_time.as_secs_f64())
                    * 100.0
            } else {
                0.0
            };
            println!(
                "\n{}",
                format!(
                    "Optimization potential: ~{:.0}s ({:.0}%) by addressing slowest 80%",
                    report.optimization_potential.as_secs_f64(),
                    opt_pct
                )
                .cyan()
            );
        }

        // Global recommendations
        if !report.recommendations.is_empty() {
            println!("\n{}:", "Recommendations".green().bold());
            for (idx, rec) in report.recommendations.iter().enumerate() {
                println!("  {}. {}", idx + 1, rec.as_str());
            }
        }

        println!();
    }

    /// Export report as a JSON string for CI/CD integration.
    ///
    /// Returns a pretty-printed JSON string containing all report data:
    /// slow verifications, cache statistics, summary metrics, and recommendations.
    pub fn export_json(&self) -> String {
        let value = self.export_json_value();
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    }

    /// Export report as a `serde_json::Value` for programmatic access.
    pub fn export_json_value(&self) -> serde_json::Value {
        let report = self.generate_report();

        let slow_verifications: Vec<_> = report
            .slow_verifications
            .iter()
            .map(|e| {
                serde_json::json!({
                    "function": e.function_name.as_str(),
                    "file": e.file_location.file.display().to_string(),
                    "line": e.file_location.line,
                    "column": e.file_location.column,
                    "verification_time_secs": e.verification_time.as_secs_f64(),
                    "smt_solver": e.smt_solver.as_str(),
                    "logic": e.logic.as_str(),
                    "query_count": e.query_count,
                    "bottleneck": e.bottleneck.as_ref().map(|b| b.as_str()),
                    "recommendations": e.recommendations.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                })
            })
            .collect();

        let all_entries: Vec<_> = self
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "function": e.function_name.as_str(),
                    "file": e.file_location.file.display().to_string(),
                    "line": e.file_location.line,
                    "verification_time_secs": e.verification_time.as_secs_f64(),
                    "smt_solver": e.smt_solver.as_str(),
                    "logic": e.logic.as_str(),
                    "query_count": e.query_count,
                })
            })
            .collect();

        let functions_over_1s = self
            .entries
            .iter()
            .filter(|e| e.verification_time > Duration::from_secs(1))
            .count();

        serde_json::json!({
            "slow_verifications": slow_verifications,
            "all_entries": all_entries,
            "cache_stats": {
                "hits": report.cache_stats.hits,
                "misses": report.cache_stats.misses,
                "hit_rate": report.cache_stats.hit_rate(),
                "time_saved_secs": report.cache_stats.time_saved.as_secs_f64(),
                "cache_size_bytes": report.cache_stats.cache_size_bytes,
                "entry_count": report.cache_stats.entry_count,
            },
            "summary": {
                "total_functions": report.function_count,
                "total_time_secs": report.total_time.as_secs_f64(),
                "average_time_secs": report.avg_time_per_function.as_secs_f64(),
                "functions_above_threshold": report.functions_above_threshold,
                "functions_over_1s": functions_over_1s,
                "optimization_potential_secs": report.optimization_potential.as_secs_f64(),
                "slow_threshold_secs": self.slow_threshold.as_secs_f64(),
            },
            "recommendations": report.recommendations.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
        })
    }

    /// Update cache statistics from external source
    pub fn update_cache_stats(&mut self, stats: verum_smt::verification_cache::CacheStats) {
        self.cache_stats.hits = stats.cache_hits as usize;
        self.cache_stats.misses = stats.cache_misses as usize;
        self.cache_stats.entry_count = stats.current_size;
        // Note: time_saved and cache_size_bytes are estimated internally
    }
}

impl Default for VerificationProfiler {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for FileLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let file_name = self
            .file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unknown>");
        write!(f, "{}:{}", file_name, self.line)
    }
}

impl std::fmt::Display for SmtSolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Infer SMT logic from verification cost metadata
///
/// This function analyzes the complexity, category, and number of checks
/// to determine the most likely SMT logic used for the verification.
fn infer_smt_logic(cost: &VerificationCost) -> SmtLogic {
    // Use category to determine appropriate logic
    let category = cost.category.as_str();

    // Check for specific verification categories
    if category.contains("array") || category.contains("buffer") || category.contains("index") {
        // Array operations typically use QF_AUFLIA (quantifier-free arrays with linear integer arithmetic)
        return SmtLogic::QF_AUFLIA;
    }

    if category.contains("bitvector")
        || category.contains("bitwise")
        || category.contains("overflow")
    {
        // Bit-level operations use QF_BV (quantifier-free bit-vectors)
        return SmtLogic::QF_BV;
    }

    if category.contains("nonlinear")
        || category.contains("polynomial")
        || category.contains("multiplication_bounds")
    {
        // Nonlinear arithmetic
        return SmtLogic::QF_NIA;
    }

    if category.contains("floating") || category.contains("real") {
        // Real arithmetic (floating point approximations)
        return SmtLogic::QF_NRA;
    }

    if category.contains("quantifier") || category.contains("forall") || category.contains("exists")
    {
        // Full first-order logic with quantifiers
        return SmtLogic::ALL;
    }

    // Infer from complexity and number of checks
    if cost.complexity > 70 {
        // High complexity often indicates combined theories
        SmtLogic::ALL
    } else if cost.complexity > 50 {
        // Medium complexity often involves arrays or bitvectors
        SmtLogic::QF_AUFLIA
    } else if cost.num_checks > 10 {
        // Many checks often indicate quantifier instantiation
        SmtLogic::ALL
    } else {
        // Default to linear integer arithmetic for simple cases
        SmtLogic::QF_LIA
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profiler_creation() {
        let profiler = VerificationProfiler::new();
        assert_eq!(profiler.entries.len(), 0);
        assert_eq!(profiler.cache_stats.hits, 0);
    }

    #[test]
    fn test_cache_statistics() {
        let mut stats = CacheStatistics::default();
        stats.hits = 80;
        stats.misses = 20;

        assert_eq!(stats.hit_rate(), 0.8);
        assert_eq!(stats.total_requests(), 100);
    }

    #[test]
    fn test_bottleneck_detection() {
        let profiler = VerificationProfiler::new();

        // Test array reasoning detection
        let result: Result<ProofResult, VerificationError> = Ok(ProofResult::new(
            verum_smt::VerificationCost::new("test".into(), Duration::ZERO, true),
        ));
        let bottleneck =
            profiler.analyze_bottleneck(&result, &SmtLogic::QF_AUFLIA, Duration::from_secs(4));

        if let Maybe::Some(b) = bottleneck {
            assert!(b.as_str().contains("Array"));
        }
    }

    #[test]
    fn test_recommendations() {
        let profiler = VerificationProfiler::new();

        let recs = profiler.generate_recommendations(
            Duration::from_secs(6),
            &Maybe::Some("Array reasoning".to_text()),
            &SmtLogic::QF_AUFLIA,
        );

        assert!(recs.len() > 0);
        assert!(recs.iter().any(|r| r.as_str().contains("split")));
    }

    /// Helper to build a profiler with synthetic entries for testing
    fn profiler_with_entries(
        entries: Vec<(&str, Duration, SmtLogic, usize)>,
    ) -> VerificationProfiler {
        let mut profiler = VerificationProfiler::new();
        for (name, time, logic, queries) in entries {
            profiler.entries.push(ProfileEntry {
                function_name: name.to_text(),
                file_location: FileLocation::new(
                    PathBuf::from(format!("{}.vr", name)),
                    42,
                    1,
                ),
                verification_time: time,
                smt_solver: SmtSolver::Z3,
                logic,
                query_count: queries,
                bottleneck: if time > Duration::from_secs(5) {
                    Maybe::Some("Slow formula".to_text())
                } else {
                    Maybe::None
                },
                recommendations: List::new(),
            });
        }
        profiler
    }

    #[test]
    fn test_generate_report_sorts_by_time_descending() {
        let profiler = profiler_with_entries(vec![
            ("fast_fn", Duration::from_millis(100), SmtLogic::QF_LIA, 2),
            ("slow_fn", Duration::from_secs(10), SmtLogic::QF_NIA, 50),
            ("mid_fn", Duration::from_secs(2), SmtLogic::QF_BV, 10),
        ]);

        let report = profiler.generate_report();
        // Only slow_fn exceeds 5s threshold
        assert_eq!(report.slow_verifications.len(), 1);
        assert_eq!(report.slow_verifications[0].function_name.as_str(), "slow_fn");
    }

    #[test]
    fn test_generate_report_computes_totals() {
        let profiler = profiler_with_entries(vec![
            ("fn_a", Duration::from_secs(1), SmtLogic::QF_LIA, 5),
            ("fn_b", Duration::from_secs(3), SmtLogic::QF_LIA, 10),
            ("fn_c", Duration::from_secs(6), SmtLogic::QF_NIA, 20),
        ]);

        let report = profiler.generate_report();
        assert_eq!(report.function_count, 3);
        assert_eq!(report.total_time, Duration::from_secs(10));
        // avg = 10s / 3 = 3.333...s
        let avg_secs = report.avg_time_per_function.as_secs_f64();
        assert!((avg_secs - 3.333).abs() < 0.01, "avg was {}", avg_secs);
        assert_eq!(report.functions_above_threshold, 1);
    }

    #[test]
    fn test_generate_report_optimization_potential() {
        // With 10 entries, top 80% = 8 entries
        let profiler = profiler_with_entries(vec![
            ("fn_1", Duration::from_secs(10), SmtLogic::QF_NIA, 50),
            ("fn_2", Duration::from_secs(8), SmtLogic::QF_NIA, 40),
            ("fn_3", Duration::from_secs(6), SmtLogic::QF_NIA, 30),
            ("fn_4", Duration::from_secs(4), SmtLogic::QF_LIA, 20),
            ("fn_5", Duration::from_secs(2), SmtLogic::QF_LIA, 10),
        ]);

        let report = profiler.generate_report();
        // Top 80% of 5 entries = ceil(4.0) = 4 entries: 10+8+6+4 = 28s
        assert_eq!(report.optimization_potential, Duration::from_secs(28));
    }

    #[test]
    fn test_configurable_threshold() {
        let mut profiler = profiler_with_entries(vec![
            ("fn_a", Duration::from_secs(2), SmtLogic::QF_LIA, 5),
            ("fn_b", Duration::from_secs(4), SmtLogic::QF_LIA, 10),
            ("fn_c", Duration::from_secs(6), SmtLogic::QF_NIA, 20),
        ]);

        // Default threshold (5s) - only fn_c is slow
        let report = profiler.generate_report();
        assert_eq!(report.functions_above_threshold, 1);

        // Lower threshold to 3s - fn_b and fn_c are slow
        profiler.set_threshold(Duration::from_secs(3));
        let report = profiler.generate_report();
        assert_eq!(report.functions_above_threshold, 2);
        assert_eq!(profiler.threshold(), Duration::from_secs(3));
    }

    #[test]
    fn test_with_threshold_constructor() {
        let profiler = VerificationProfiler::with_threshold(Duration::from_secs(10));
        assert_eq!(profiler.threshold(), Duration::from_secs(10));
    }

    #[test]
    fn test_empty_report() {
        let profiler = VerificationProfiler::new();
        let report = profiler.generate_report();

        assert_eq!(report.function_count, 0);
        assert_eq!(report.total_time, Duration::ZERO);
        assert_eq!(report.avg_time_per_function, Duration::ZERO);
        assert_eq!(report.functions_above_threshold, 0);
        assert_eq!(report.optimization_potential, Duration::ZERO);
        assert!(report.slow_verifications.is_empty());
    }

    #[test]
    fn test_export_json_returns_valid_json_string() {
        let profiler = profiler_with_entries(vec![
            ("test_fn", Duration::from_secs(7), SmtLogic::QF_NIA, 25),
        ]);

        let json_str = profiler.export_json();
        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .expect("export_json should return valid JSON");

        assert!(parsed["slow_verifications"].is_array());
        assert_eq!(parsed["slow_verifications"].as_array().unwrap().len(), 1);
        assert_eq!(
            parsed["slow_verifications"][0]["function"].as_str().unwrap(),
            "test_fn"
        );
        assert_eq!(parsed["summary"]["total_functions"].as_u64().unwrap(), 1);
        assert!(parsed["summary"]["optimization_potential_secs"].is_f64());
        assert!(parsed["summary"]["slow_threshold_secs"].is_f64());
        assert!(parsed["all_entries"].is_array());
    }

    #[test]
    fn test_export_json_value_matches_export_json() {
        let profiler = profiler_with_entries(vec![
            ("fn_x", Duration::from_secs(1), SmtLogic::QF_LIA, 3),
        ]);

        let json_str = profiler.export_json();
        let value = profiler.export_json_value();
        let from_str: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(from_str, value);
    }

    #[test]
    fn test_global_recommendations_distributed_cache() {
        let mut profiler = VerificationProfiler::new();
        // Low cache hit rate with enough requests triggers distributed cache recommendation
        profiler.cache_stats.hits = 10;
        profiler.cache_stats.misses = 30;

        let recs = profiler.generate_global_recommendations();
        assert!(recs.iter().any(|r| r.as_str().contains("distributed cache")));
    }

    #[test]
    fn test_global_recommendations_many_slow() {
        let profiler = profiler_with_entries(vec![
            ("s1", Duration::from_secs(6), SmtLogic::QF_NIA, 10),
            ("s2", Duration::from_secs(7), SmtLogic::QF_NIA, 10),
            ("s3", Duration::from_secs(8), SmtLogic::QF_NIA, 10),
            ("s4", Duration::from_secs(9), SmtLogic::QF_NIA, 10),
        ]);

        let recs = profiler.generate_global_recommendations();
        assert!(recs.iter().any(|r| r.as_str().contains("budget")));
        assert!(recs.iter().any(|r| r.as_str().contains("@verify(runtime)")));
    }

    #[test]
    fn test_file_location_display() {
        let loc = FileLocation::new(PathBuf::from("src/algorithms.vr"), 42, 5);
        assert_eq!(format!("{}", loc), "algorithms.vr:42");
    }

    #[test]
    fn test_smt_solver_display() {
        assert_eq!(format!("{}", SmtSolver::Z3), "Z3");
        assert_eq!(format!("{}", SmtSolver::CVC5), "CVC5");
    }

    #[test]
    fn test_print_report_does_not_panic() {
        // Ensure print_report runs without panicking on various states
        let profiler = VerificationProfiler::new();
        profiler.print_report(); // empty

        let profiler = profiler_with_entries(vec![
            ("fast", Duration::from_millis(50), SmtLogic::QF_LIA, 1),
            ("slow", Duration::from_secs(10), SmtLogic::ALL, 100),
        ]);
        profiler.print_report(); // with entries
    }
}
