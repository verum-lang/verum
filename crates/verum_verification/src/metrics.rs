//! Code Metrics Collection for Transition Analysis
//!
//! This module implements comprehensive code metrics collection for the
//! gradual verification transition system. It provides both static analysis
//! metrics (from AST/CFG) and external data integration (git history,
//! coverage, profiling).
//!
//! Metrics drive the gradual verification transition system: code with high
//! cyclomatic complexity, frequent changes, or low coverage stays at runtime
//! level, while stable, well-tested code is recommended for static/proof.
//!
//! # Features
//!
//! - Cyclomatic complexity calculation from CFG
//! - Dependency analysis from imports/function calls
//! - Invariant counting from contracts
//! - Loop nesting depth analysis
//! - Assertion density computation
//! - Git history integration for change frequency
//! - Coverage data loading (lcov format)
//! - Profiling data integration (perf/callgrind)
//!
//! # Example
//!
//! ```rust,ignore
//! use verum_verification::metrics::{CodeMetricsCollector, EnhancedCodeMetrics};
//! use verum_ast::decl::FunctionDecl;
//!
//! let mut collector = CodeMetricsCollector::new();
//! collector.load_coverage(Path::new("coverage.lcov")).ok();
//!
//! let func: FunctionDecl = /* ... */;
//! let metrics = collector.analyze_function(&func);
//! println!("Complexity: {}", metrics.cyclomatic_complexity);
//! ```

use crate::cbgr_elimination::ControlFlowGraph as EscapeCFG;
use crate::transition::CodeMetrics;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;
use verum_ast::Module;
use verum_ast::decl::{FunctionBody, FunctionDecl, Item, ItemKind};
use verum_ast::expr::{BinOp, Block, Expr, ExprKind};
use verum_ast::stmt::{Stmt, StmtKind};
use verum_common::{List, Maybe, Text, ToText};

// =============================================================================
// Error Types
// =============================================================================

/// Errors that can occur during metrics collection
#[derive(Debug, Error)]
pub enum MetricsError {
    /// IO error reading files
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// Failed to parse coverage file
    #[error("Coverage parse error: {0}")]
    CoverageParse(Text),

    /// Failed to parse profiling data
    #[error("Profiling parse error: {0}")]
    ProfilingParse(Text),

    /// Git operation failed
    #[error("Git error: {0}")]
    Git(Text),

    /// Invalid metric data
    #[error("Invalid metric: {0}")]
    InvalidMetric(Text),
}

/// Result type for metrics operations
pub type MetricsResult<T> = Result<T, MetricsError>;

// =============================================================================
// Enhanced Code Metrics
// =============================================================================

/// Enhanced code metrics for transition analysis
///
/// Comprehensive code characteristics for automated transition recommendations
/// between verification levels. Includes cyclomatic complexity, dependency count,
/// change frequency, test coverage, and assertion density.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnhancedCodeMetrics {
    // === Basic metrics (from CodeMetrics) ===
    /// Test coverage (0.0 to 1.0)
    pub test_coverage: f64,

    /// Change frequency (changes per week)
    pub change_frequency_per_week: f64,

    /// Criticality score (0 to 10)
    pub criticality_score: u32,

    /// Execution frequency (calls per second in production)
    pub execution_frequency: f64,

    // === NEW fields per spec ===
    /// Cyclomatic complexity (number of linearly independent paths)
    pub cyclomatic_complexity: u32,

    /// Lines of code in the function
    pub lines_of_code: u32,

    /// Number of dependencies (imported modules, called functions)
    pub dependency_count: u32,

    /// Number of invariants/contracts in the function
    pub invariant_count: u32,

    /// Whether the function contains unsafe blocks
    pub has_unsafe_blocks: bool,

    /// Maximum loop nesting depth
    pub loop_nesting_depth: u32,

    /// Assertion density (asserts per LOC)
    pub assertion_density: f64,

    // === Additional analysis fields ===
    /// Number of branches (if, match, etc.)
    pub branch_count: u32,

    /// Number of loop constructs
    pub loop_count: u32,

    /// Number of function calls
    pub call_count: u32,

    /// Number of reference operations
    pub reference_count: u32,

    /// Proof complexity estimate (SMT query size)
    pub proof_complexity: u32,

    /// Whether code has complex predicates
    pub has_complex_predicates: bool,

    /// Function name
    pub function_name: Text,

    /// File path (if known)
    pub file_path: Maybe<Text>,
}

impl EnhancedCodeMetrics {
    /// Create new metrics with default values
    pub fn new(function_name: impl Into<Text>) -> Self {
        Self {
            test_coverage: 0.0,
            change_frequency_per_week: 10.0, // Default: unstable
            criticality_score: 5,
            execution_frequency: 0.0,
            cyclomatic_complexity: 1, // Minimum complexity
            lines_of_code: 0,
            dependency_count: 0,
            invariant_count: 0,
            has_unsafe_blocks: false,
            loop_nesting_depth: 0,
            assertion_density: 0.0,
            branch_count: 0,
            loop_count: 0,
            call_count: 0,
            reference_count: 0,
            proof_complexity: 50,
            has_complex_predicates: false,
            function_name: function_name.into(),
            file_path: Maybe::None,
        }
    }

    /// Check if code is stable (low change frequency)
    pub fn is_stable(&self) -> bool {
        self.change_frequency_per_week < 1.0
    }

    /// Check if code is very stable
    pub fn is_very_stable(&self) -> bool {
        self.change_frequency_per_week < 0.1
    }

    /// Check if code has good test coverage
    pub fn has_good_tests(&self) -> bool {
        self.test_coverage > 0.90
    }

    /// Check if code is critical
    pub fn is_critical(&self) -> bool {
        self.criticality_score >= 8
    }

    /// Check if code has high complexity
    pub fn is_complex(&self) -> bool {
        self.cyclomatic_complexity > 10
    }

    /// Check if code has deep nesting
    pub fn has_deep_nesting(&self) -> bool {
        self.loop_nesting_depth > 3
    }

    /// Calculate maintainability index (0-100)
    ///
    /// Based on Halstead and McCabe metrics.
    pub fn maintainability_index(&self) -> f64 {
        let loc = self.lines_of_code.max(1) as f64;
        let cc = self.cyclomatic_complexity.max(1) as f64;

        // Simplified maintainability formula
        // MI = 171 - 5.2 * ln(V) - 0.23 * G - 16.2 * ln(LOC)
        // Where V is Halstead volume, G is cyclomatic complexity
        // We approximate V ~ LOC * ln(LOC)
        let halstead_approx = loc * loc.ln().max(1.0);

        let mi = 171.0 - 5.2 * halstead_approx.ln() - 0.23 * cc - 16.2 * loc.ln();

        // Normalize to 0-100 scale
        (mi.max(0.0) / 171.0 * 100.0).min(100.0)
    }

    /// Calculate risk score for verification transition (0-10)
    ///
    /// Higher scores indicate more risk in transitioning.
    pub fn transition_risk_score(&self) -> f64 {
        let mut risk = 0.0;

        // High complexity increases risk
        if self.cyclomatic_complexity > 20 {
            risk += 3.0;
        } else if self.cyclomatic_complexity > 10 {
            risk += 1.5;
        }

        // Deep nesting increases risk
        if self.loop_nesting_depth > 4 {
            risk += 2.0;
        } else if self.loop_nesting_depth > 2 {
            risk += 1.0;
        }

        // Unsafe blocks are risky
        if self.has_unsafe_blocks {
            risk += 2.0;
        }

        // Complex predicates increase proof difficulty
        if self.has_complex_predicates {
            risk += 1.5;
        }

        // Low test coverage is risky
        if self.test_coverage < 0.5 {
            risk += 2.0;
        } else if self.test_coverage < 0.8 {
            risk += 1.0;
        }

        // High change frequency is risky
        if self.change_frequency_per_week > 5.0 {
            risk += 2.0;
        } else if self.change_frequency_per_week > 1.0 {
            risk += 1.0;
        }

        if risk > 10.0 { 10.0 } else { risk }
    }

    /// Convert to basic CodeMetrics for transition analyzer compatibility
    pub fn to_code_metrics(&self) -> CodeMetrics {
        CodeMetrics {
            test_coverage: self.test_coverage,
            change_frequency_per_week: self.change_frequency_per_week,
            lines_of_code: self.lines_of_code as usize,
            cyclomatic_complexity: self.cyclomatic_complexity as usize,
            proof_complexity: self.proof_complexity as usize,
            execution_frequency: self.execution_frequency,
            criticality_score: self.criticality_score as u8,
            has_loops: self.loop_count > 0,
            has_complex_predicates: self.has_complex_predicates,
            dependency_count: self.dependency_count,
            invariant_count: self.invariant_count,
            has_unsafe_blocks: self.has_unsafe_blocks,
            loop_nesting_depth: self.loop_nesting_depth,
            assertion_density: self.assertion_density,
        }
    }
}

impl Default for EnhancedCodeMetrics {
    fn default() -> Self {
        Self::new("unknown")
    }
}

impl From<EnhancedCodeMetrics> for CodeMetrics {
    fn from(enhanced: EnhancedCodeMetrics) -> Self {
        enhanced.to_code_metrics()
    }
}

// =============================================================================
// Git History Data
// =============================================================================

/// Git history information for change frequency analysis
#[derive(Debug, Clone, Default)]
pub struct GitHistory {
    /// File path -> list of commit timestamps
    file_commits: HashMap<PathBuf, List<i64>>,
    /// File path -> list of authors
    file_authors: HashMap<PathBuf, HashSet<Text>>,
    /// Repository root path
    repo_root: Option<PathBuf>,
}

impl GitHistory {
    /// Create a new Git history tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Load git history from repository
    ///
    /// Uses git commands to extract commit history, author information,
    /// and change frequency for verification metrics.
    ///
    /// # Algorithm
    ///
    /// 1. Verify git repository exists at path
    /// 2. Run `git log` to extract commits for each file
    /// 3. Parse commit timestamps and authors
    /// 4. Build per-file commit and author maps
    ///
    /// # Performance
    ///
    /// - Initial load: O(n * log(commits)) where n = files
    /// - Uses batch git commands to minimize process spawning
    /// - Caches results for subsequent queries
    pub fn load_from_repo(repo_path: &Path) -> MetricsResult<Self> {
        use std::process::Command;

        let mut history = Self::new();
        history.repo_root = Some(repo_path.to_path_buf());

        // Verify this is a git repository
        let git_dir = repo_path.join(".git");
        if !git_dir.exists() {
            return Err(MetricsError::Git(
                format!("Not a git repository: {}", repo_path.display()).into(),
            ));
        }

        // Get list of tracked files
        let output = Command::new("git")
            .args(["ls-files"])
            .current_dir(repo_path)
            .output()
            .map_err(|e| MetricsError::Git(format!("Failed to run git ls-files: {}", e).into()))?;

        if !output.status.success() {
            return Err(MetricsError::Git(
                format!(
                    "git ls-files failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
                .into(),
            ));
        }

        let files: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| repo_path.join(line))
            .collect();

        // For each file, get commit history
        // Use batch processing for efficiency
        for file in files {
            if !file.exists() {
                continue;
            }

            // Get file path relative to repo root
            let rel_path = file.strip_prefix(repo_path).unwrap_or(&file);

            // Get commit log for this file: timestamp and author
            // Format: %at (Unix timestamp) %ae (author email)
            let log_output = Command::new("git")
                .args([
                    "log",
                    "--follow",
                    "--format=%at|%ae",
                    "--",
                    rel_path.to_str().unwrap_or_default(),
                ])
                .current_dir(repo_path)
                .output();

            if let Ok(log_result) = log_output {
                if log_result.status.success() {
                    for line in String::from_utf8_lossy(&log_result.stdout).lines() {
                        let parts: Vec<&str> = line.split('|').collect();
                        if parts.len() >= 2 {
                            // Parse timestamp
                            if let Ok(timestamp) = parts[0].trim().parse::<i64>() {
                                let author = if parts.len() > 1 {
                                    Some(parts[1].trim())
                                } else {
                                    None
                                };
                                history.add_commit(&file, timestamp, author);
                            }
                        }
                    }
                }
            }
        }

        Ok(history)
    }

    /// Load git history for a specific set of files
    ///
    /// More efficient when only analyzing a subset of files.
    pub fn load_for_files(repo_path: &Path, files: &[PathBuf]) -> MetricsResult<Self> {
        use std::process::Command;

        let mut history = Self::new();
        history.repo_root = Some(repo_path.to_path_buf());

        for file in files {
            if !file.exists() {
                continue;
            }

            // Get file path relative to repo root
            let rel_path = file.strip_prefix(repo_path).unwrap_or(file);

            // Get commit log for this file
            let log_output = Command::new("git")
                .args([
                    "log",
                    "--follow",
                    "--format=%at|%ae",
                    "--",
                    rel_path.to_str().unwrap_or_default(),
                ])
                .current_dir(repo_path)
                .output();

            if let Ok(log_result) = log_output {
                if log_result.status.success() {
                    for line in String::from_utf8_lossy(&log_result.stdout).lines() {
                        let parts: Vec<&str> = line.split('|').collect();
                        if !parts.is_empty() {
                            if let Ok(timestamp) = parts[0].trim().parse::<i64>() {
                                let author = if parts.len() > 1 {
                                    Some(parts[1].trim())
                                } else {
                                    None
                                };
                                history.add_commit(file, timestamp, author);
                            }
                        }
                    }
                }
            }
        }

        Ok(history)
    }

    /// Get the age of the oldest commit for a file (in days)
    pub fn file_age_days(&self, file: &Path) -> Option<f64> {
        self.file_commits.get(file).and_then(|commits| {
            commits.iter().min().map(|oldest| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                (now - oldest) as f64 / (24.0 * 60.0 * 60.0)
            })
        })
    }

    /// Get the age of the most recent commit for a file (in days)
    pub fn time_since_last_change(&self, file: &Path) -> Option<f64> {
        self.file_commits.get(file).and_then(|commits| {
            commits.iter().max().map(|newest| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                (now - newest) as f64 / (24.0 * 60.0 * 60.0)
            })
        })
    }

    /// Calculate a "hotness" score for a file based on recent activity
    ///
    /// Score considers:
    /// - Number of recent commits (last 30 days weighted higher)
    /// - Number of unique authors
    /// - Change frequency
    ///
    /// Returns a score from 0.0 (cold) to 1.0 (hot)
    pub fn hotness_score(&self, file: &Path) -> f64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let thirty_days_ago = now - (30 * 24 * 60 * 60);
        let ninety_days_ago = now - (90 * 24 * 60 * 60);

        let commits = self.file_commits.get(file);
        if commits.is_none() {
            return 0.0;
        }
        let commits = commits.unwrap();

        // Count recent commits with decay
        let recent_30 = commits.iter().filter(|&&t| t > thirty_days_ago).count() as f64;
        let recent_90 = commits
            .iter()
            .filter(|&&t| t > ninety_days_ago && t <= thirty_days_ago)
            .count() as f64;
        let older = commits.len() as f64 - recent_30 - recent_90;

        // Weighted score: recent commits matter more
        let commit_score =
            (recent_30 * 1.0 + recent_90 * 0.5 + older * 0.1) / (commits.len() as f64 + 1.0);

        // Author diversity (more authors = hotter)
        let authors = self.author_count(file) as f64;
        let author_score = (authors / 5.0).min(1.0); // Cap at 5 authors

        // Combined score (weighted average)
        (commit_score * 0.7 + author_score * 0.3).min(1.0)
    }

    /// Add commit information for a file
    pub fn add_commit(&mut self, file: &Path, timestamp: i64, author: Option<&str>) {
        self.file_commits
            .entry(file.to_path_buf())
            .or_default()
            .push(timestamp);

        if let Some(author) = author {
            self.file_authors
                .entry(file.to_path_buf())
                .or_default()
                .insert(author.to_text());
        }
    }

    /// Get commit count for a file
    pub fn commit_count(&self, file: &Path) -> usize {
        self.file_commits.get(file).map(|c| c.len()).unwrap_or(0)
    }

    /// Get number of unique authors for a file
    pub fn author_count(&self, file: &Path) -> usize {
        self.file_authors.get(file).map(|a| a.len()).unwrap_or(0)
    }

    /// Calculate change frequency (changes per week) over a time period
    pub fn change_frequency(&self, file: &Path, weeks: f64) -> f64 {
        let commits = self.commit_count(file);
        if weeks > 0.0 {
            commits as f64 / weeks
        } else {
            commits as f64
        }
    }
}

// =============================================================================
// Coverage Data
// =============================================================================

/// Test coverage data from lcov format
#[derive(Debug, Clone, Default)]
pub struct CoverageData {
    /// Function name -> (lines_hit, lines_total)
    function_coverage: HashMap<Text, (u32, u32)>,
    /// File path -> line coverage
    file_coverage: HashMap<PathBuf, LineCoverage>,
}

/// Line coverage information
#[derive(Debug, Clone, Default)]
pub struct LineCoverage {
    /// Line number -> hit count
    lines: HashMap<u32, u32>,
    /// Total lines in file
    total_lines: u32,
}

impl CoverageData {
    /// Create new empty coverage data
    pub fn new() -> Self {
        Self::default()
    }

    /// Load coverage from lcov format file
    ///
    /// LCOV format:
    /// ```text
    /// SF:<file path>
    /// FN:<line>,<function name>
    /// FNDA:<hit count>,<function name>
    /// DA:<line>,<hit count>
    /// LF:<lines found>
    /// LH:<lines hit>
    /// end_of_record
    /// ```
    pub fn load_lcov(path: &Path) -> MetricsResult<Self> {
        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);

        let mut coverage = Self::new();
        let mut current_file: Option<PathBuf> = None;
        let mut current_coverage = LineCoverage::default();
        let mut current_functions: HashMap<Text, (u32, u32)> = HashMap::new();

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.starts_with("SF:") {
                // Start of file
                current_file = Some(PathBuf::from(&line[3..]));
                current_coverage = LineCoverage::default();
                current_functions.clear();
            } else if line.starts_with("DA:") {
                // Line data: DA:<line>,<hit count>
                let parts: List<&str> = line[3..].split(',').collect();
                if parts.len() >= 2
                    && let (Ok(line_no), Ok(hits)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                {
                    current_coverage.lines.insert(line_no, hits);
                }
            } else if line.starts_with("FN:") {
                // Function: FN:<line>,<name>
                let parts: List<&str> = line[3..].split(',').collect();
                if parts.len() >= 2 {
                    current_functions.insert(parts[1].to_text(), (0, 0));
                }
            } else if line.starts_with("FNDA:") {
                // Function data: FNDA:<hit count>,<name>
                let parts: List<&str> = line[5..].split(',').collect();
                if parts.len() >= 2
                    && let Ok(hits) = parts[0].parse::<u32>()
                    && let Some(entry) = current_functions.get_mut(parts[1])
                {
                    entry.0 = hits;
                    entry.1 = 1; // Mark as having data
                }
            } else if line.starts_with("LF:") {
                // Lines found
                if let Ok(total) = line[3..].parse::<u32>() {
                    current_coverage.total_lines = total;
                }
            } else if line == "end_of_record" {
                // End of record - save data
                if let Some(ref file) = current_file {
                    coverage
                        .file_coverage
                        .insert(file.clone(), current_coverage.clone());
                }
                for (func_name, data) in &current_functions {
                    coverage.function_coverage.insert(func_name.clone(), *data);
                }
                current_file = None;
            }
        }

        Ok(coverage)
    }

    /// Get coverage ratio for a function (0.0 to 1.0)
    pub fn function_coverage(&self, func_name: &str) -> f64 {
        if let Some((hits, total)) = self.function_coverage.get(func_name)
            && *total > 0
        {
            return *hits as f64 / *total as f64;
        }
        0.0
    }

    /// Get line coverage for a file
    pub fn file_coverage(&self, path: &Path) -> f64 {
        if let Some(coverage) = self.file_coverage.get(path)
            && coverage.total_lines > 0
        {
            let hit_lines = coverage.lines.values().filter(|&&c| c > 0).count();
            return hit_lines as f64 / coverage.total_lines as f64;
        }
        0.0
    }

    /// Get coverage for a specific line
    pub fn line_coverage(&self, path: &Path, line: u32) -> Option<u32> {
        self.file_coverage
            .get(path)
            .and_then(|c| c.lines.get(&line).copied())
    }
}

// =============================================================================
// Profiling Data
// =============================================================================

/// Profiling data from perf/callgrind
#[derive(Debug, Clone, Default)]
pub struct ProfilingData {
    /// Function name -> call count
    call_counts: HashMap<Text, u64>,
    /// Function name -> total cycles/time
    execution_time: HashMap<Text, u64>,
    /// Total profiling duration (seconds)
    total_duration: f64,
}

impl ProfilingData {
    /// Create new empty profiling data
    pub fn new() -> Self {
        Self::default()
    }

    /// Load profiling data from callgrind format
    ///
    /// Callgrind format:
    /// ```text
    /// fn=<function name>
    /// <line> <count>
    /// ```
    pub fn load_callgrind(path: &Path) -> MetricsResult<Self> {
        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);

        let mut profiling = Self::new();
        let mut current_function: Option<Text> = None;

        for line in reader.lines() {
            let line = line?;
            let line = line.trim();

            if line.starts_with("fn=") {
                current_function = Some(line[3..].to_text());
            } else if line.starts_with("summary:") {
                // Total events
                if let Ok(total) = line[8..].trim().parse::<u64>() {
                    profiling.total_duration = total as f64;
                }
            } else if let Some(ref func) = current_function {
                // Try to parse as "<line> <count>"
                let parts: List<&str> = line.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(count) = parts[1].parse::<u64>()
                {
                    *profiling.call_counts.entry(func.clone()).or_default() += 1;
                    *profiling.execution_time.entry(func.clone()).or_default() += count;
                }
            }
        }

        Ok(profiling)
    }

    /// Load profiling data from perf format
    pub fn load_perf(path: &Path) -> MetricsResult<Self> {
        let content = fs::read_to_string(path)?;
        let mut profiling = Self::new();

        // Simple perf format parsing
        // Lines like: "12.34%  program  [.] function_name"
        for line in content.lines() {
            let parts: List<&str> = line.split_whitespace().collect();
            if parts.len() >= 4
                && let Ok(pct) = parts[0].trim_end_matches('%').parse::<f64>()
            {
                let func_name = parts[3].to_text();
                let calls = (pct * 100.0) as u64; // Approximate from percentage
                profiling.call_counts.insert(func_name.clone(), calls);
                profiling.execution_time.insert(func_name, calls);
            }
        }

        Ok(profiling)
    }

    /// Get execution frequency (calls per second)
    pub fn execution_frequency(&self, func_name: &str) -> f64 {
        if self.total_duration > 0.0 {
            let calls = self.call_counts.get(func_name).copied().unwrap_or(0);
            calls as f64 / self.total_duration
        } else {
            0.0
        }
    }

    /// Get total execution time for a function
    pub fn total_time(&self, func_name: &str) -> u64 {
        self.execution_time.get(func_name).copied().unwrap_or(0)
    }

    /// Check if function is a hot path (high execution frequency)
    pub fn is_hot_path(&self, func_name: &str) -> bool {
        self.execution_frequency(func_name) > 1000.0
    }
}

// =============================================================================
// AST Analysis Visitor
// =============================================================================

/// Visitor for collecting metrics from AST
struct MetricsVisitor {
    /// Current metrics being collected
    metrics: EnhancedCodeMetrics,
    /// Current nesting depth for loops
    current_loop_depth: u32,
    /// Set of called functions
    called_functions: HashSet<Text>,
    /// Count of assertions
    assertion_count: u32,
}

impl MetricsVisitor {
    fn new(function_name: impl Into<Text>) -> Self {
        Self {
            metrics: EnhancedCodeMetrics::new(function_name),
            current_loop_depth: 0,
            called_functions: HashSet::new(),
            assertion_count: 0,
        }
    }

    /// Visit a function declaration
    fn visit_function(&mut self, func: &FunctionDecl) {
        // Count parameters as potential dependencies
        self.metrics.dependency_count += func.params.len() as u32;

        // Analyze attributes for unsafe, contracts, etc.
        for attr in func.attributes.iter() {
            let attr_str = format!("{:?}", attr);
            if attr_str.contains("unsafe") {
                self.metrics.has_unsafe_blocks = true;
            }
            if attr_str.contains("invariant") || attr_str.contains("contract") {
                self.metrics.invariant_count += 1;
            }
        }

        // Analyze function body
        if let Some(body) = &func.body {
            match body {
                FunctionBody::Block(block) => {
                    self.visit_block(block);
                }
                FunctionBody::Expr(expr) => {
                    self.visit_expr(expr);
                }
            }
        }

        // Calculate assertion density
        if self.metrics.lines_of_code > 0 {
            self.metrics.assertion_density =
                self.assertion_count as f64 / self.metrics.lines_of_code as f64;
        }

        // Calculate proof complexity estimate
        self.metrics.proof_complexity = self.estimate_proof_complexity();
    }

    /// Visit a block
    fn visit_block(&mut self, block: &Block) {
        for stmt in block.stmts.iter() {
            self.visit_stmt(stmt);
        }

        if let Some(expr) = &block.expr {
            self.visit_expr(expr);
        }
    }

    /// Visit a statement
    fn visit_stmt(&mut self, stmt: &Stmt) {
        self.metrics.lines_of_code += 1;

        match &stmt.kind {
            StmtKind::Let { value, .. } => {
                if let Some(expr) = value {
                    self.visit_expr(expr);
                }
            }
            StmtKind::LetElse {
                value, else_block, ..
            } => {
                self.visit_expr(value);
                self.visit_block(else_block);
                self.metrics.branch_count += 1;
                self.metrics.cyclomatic_complexity += 1;
            }
            StmtKind::Expr { expr, .. } => {
                self.visit_expr(expr);
            }
            StmtKind::Defer(expr) => {
                self.visit_expr(expr);
            }
            StmtKind::Errdefer(expr) => {
                self.visit_expr(expr);
            }
            StmtKind::Item(item) => {
                self.visit_item(item);
            }
            StmtKind::Empty => {}
            StmtKind::Provide { value, .. } => {
                self.visit_expr(value);
            }
            StmtKind::ProvideScope { value, block, .. } => {
                self.visit_expr(value);
                self.visit_expr(block);
            }
        }
    }

    /// Visit an expression
    fn visit_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.metrics.branch_count += 1;
                self.metrics.cyclomatic_complexity += 1;

                // Visit condition
                for cond in condition.conditions.iter() {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => self.visit_expr(e),
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.visit_expr(value);
                            self.metrics.branch_count += 1;
                        }
                    }
                }

                self.visit_block(then_branch);

                if let Some(else_expr) = else_branch {
                    self.visit_expr(else_expr);
                    self.metrics.branch_count += 1;
                    self.metrics.cyclomatic_complexity += 1;
                }
            }
            ExprKind::While {
                label: _,
                condition,
                body,
                invariants: _,
                decreases: _,
            } => {
                self.metrics.loop_count += 1;
                self.metrics.cyclomatic_complexity += 1;
                self.current_loop_depth += 1;
                self.metrics.loop_nesting_depth =
                    self.metrics.loop_nesting_depth.max(self.current_loop_depth);

                self.visit_expr(condition);
                self.visit_block(body);

                self.current_loop_depth -= 1;
            }
            ExprKind::For {
                label: _,
                iter,
                body,
                ..
            } => {
                self.metrics.loop_count += 1;
                self.metrics.cyclomatic_complexity += 1;
                self.current_loop_depth += 1;
                self.metrics.loop_nesting_depth =
                    self.metrics.loop_nesting_depth.max(self.current_loop_depth);

                self.visit_expr(iter);
                self.visit_block(body);

                self.current_loop_depth -= 1;
            }
            ExprKind::Loop {
                label: _,
                body,
                invariants: _,
            } => {
                self.metrics.loop_count += 1;
                self.metrics.cyclomatic_complexity += 1;
                self.current_loop_depth += 1;
                self.metrics.loop_nesting_depth =
                    self.metrics.loop_nesting_depth.max(self.current_loop_depth);

                self.visit_block(body);

                self.current_loop_depth -= 1;
            }
            ExprKind::Match { expr, arms } => {
                self.visit_expr(expr);
                self.metrics.branch_count += arms.len() as u32;
                self.metrics.cyclomatic_complexity += arms.len().saturating_sub(1) as u32;

                for arm in arms.iter() {
                    if let Some(guard) = &arm.guard {
                        self.visit_expr(guard);
                        self.metrics.cyclomatic_complexity += 1;
                    }
                    self.visit_expr(&arm.body);
                }
            }
            ExprKind::Call { func, args, .. } => {
                self.metrics.call_count += 1;

                // Track called function name
                if let ExprKind::Path(path) = &func.kind {
                    let name = path
                        .segments
                        .iter()
                        .map(|s| format!("{:?}", s))
                        .collect::<List<_>>()
                        .join(".");
                    self.called_functions.insert(name.clone());

                    // Check for assertions
                    if name.contains("assert") || name.contains("debug_assert") {
                        self.assertion_count += 1;
                    }
                }

                self.visit_expr(func);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }
            ExprKind::MethodCall {
                receiver,
                args,
                method,
                ..
            } => {
                self.metrics.call_count += 1;
                self.called_functions.insert(method.as_str().to_text());

                self.visit_expr(receiver);
                for arg in args.iter() {
                    self.visit_expr(arg);
                }
            }
            ExprKind::Binary { op, left, right } => {
                self.visit_expr(left);
                self.visit_expr(right);

                // Complex predicates involve logical operators
                if matches!(op, BinOp::And | BinOp::Or | BinOp::Imply) {
                    self.metrics.has_complex_predicates = true;
                }
            }
            ExprKind::Unary { expr, op } => {
                self.visit_expr(expr);

                // Check for reference operations
                if matches!(
                    op,
                    verum_ast::expr::UnOp::Ref
                        | verum_ast::expr::UnOp::RefMut
                        | verum_ast::expr::UnOp::Deref
                ) {
                    self.metrics.reference_count += 1;
                }
            }
            ExprKind::Block(block) => {
                self.visit_block(block);
            }
            ExprKind::Return(maybe_expr) => {
                if let Some(expr) = maybe_expr {
                    self.visit_expr(expr);
                }
            }
            ExprKind::Break { label: _, value } => {
                if let Some(expr) = value {
                    self.visit_expr(expr);
                }
            }
            ExprKind::Continue { label: _ } => {
                // Continue has no associated expression
            }
            ExprKind::Closure { body, .. } => {
                self.visit_expr(body);
            }
            ExprKind::Array(arr_expr) => match arr_expr {
                verum_ast::expr::ArrayExpr::List(exprs) => {
                    for expr in exprs.iter() {
                        self.visit_expr(expr);
                    }
                }
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.visit_expr(value);
                    self.visit_expr(count);
                }
            },
            ExprKind::Tuple(exprs) => {
                for expr in exprs.iter() {
                    self.visit_expr(expr);
                }
            }
            ExprKind::Index { expr, index } => {
                self.visit_expr(expr);
                self.visit_expr(index);
            }
            ExprKind::Field { expr, .. } => {
                self.visit_expr(expr);
            }
            ExprKind::Record { fields, base, .. } => {
                for field in fields.iter() {
                    if let Some(ref value) = field.value {
                        self.visit_expr(value);
                    }
                }
                if let Some(base_expr) = base {
                    self.visit_expr(base_expr);
                }
            }
            ExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.visit_expr(s);
                }
                if let Some(e) = end {
                    self.visit_expr(e);
                }
            }
            ExprKind::Try(expr) | ExprKind::Await(expr) | ExprKind::Paren(expr) => {
                self.visit_expr(expr);
            }
            ExprKind::Unsafe(block) => {
                self.metrics.has_unsafe_blocks = true;
                self.visit_block(block);
            }
            ExprKind::Cast { expr, .. } => {
                self.visit_expr(expr);
            }
            _ => {
                // Handle remaining expression kinds
            }
        }
    }

    /// Visit an item
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Function(_) => {
                self.metrics.dependency_count += 1;
            }
            ItemKind::Mount(_) => {
                self.metrics.dependency_count += 1;
            }
            _ => {}
        }
    }

    /// Estimate proof complexity based on collected metrics
    fn estimate_proof_complexity(&self) -> u32 {
        let mut complexity: u32 = 0;

        // Base complexity from cyclomatic complexity
        complexity += self.metrics.cyclomatic_complexity * 10;

        // Loop nesting significantly increases complexity
        complexity += self.metrics.loop_nesting_depth.pow(2) * 20;

        // Each branch adds complexity
        complexity += self.metrics.branch_count * 5;

        // Function calls add uncertainty
        complexity += self.metrics.call_count * 3;

        // Complex predicates are harder to prove
        if self.metrics.has_complex_predicates {
            complexity += 50;
        }

        // Unsafe blocks require manual verification
        if self.metrics.has_unsafe_blocks {
            complexity += 100;
        }

        complexity
    }

    /// Get the collected metrics
    fn finish(self) -> EnhancedCodeMetrics {
        let mut metrics = self.metrics;
        metrics.dependency_count += self.called_functions.len() as u32;
        metrics
    }
}

// =============================================================================
// CFG-based Analysis
// =============================================================================

/// Calculate cyclomatic complexity from a control flow graph
///
/// McCabe's cyclomatic complexity: M = E - N + 2P
/// Where:
/// - E = number of edges
/// - N = number of nodes
/// - P = number of connected components (usually 1 for a function)
pub fn calculate_cyclomatic_complexity(cfg: &EscapeCFG) -> u32 {
    let nodes = cfg.blocks.len() as i32;
    let mut edges: i32 = 0;

    for block in cfg.blocks.values() {
        edges += block.successors.len() as i32;
    }

    // M = E - N + 2P (P = 1 for single function)
    let complexity = edges - nodes + 2;
    complexity.max(1) as u32
}

/// Analyze loop nesting depth from CFG
pub fn analyze_loop_nesting(cfg: &EscapeCFG) -> u32 {
    let mut max_depth = 0;

    for scope in cfg.scopes.values() {
        if scope.is_loop {
            // Count parent loop scopes
            let mut depth = 1;
            let mut current = scope.parent;
            while let Some(parent_id) = current {
                if let Some(parent_scope) = cfg.scopes.get(&parent_id) {
                    if parent_scope.is_loop {
                        depth += 1;
                    }
                    current = parent_scope.parent;
                } else {
                    break;
                }
            }
            max_depth = max_depth.max(depth);
        }
    }

    max_depth
}

// =============================================================================
// Code Metrics Collector
// =============================================================================

/// Main metrics collector for transition analysis
///
/// Collects comprehensive code metrics from multiple sources:
/// - Static analysis of AST/CFG
/// - Git history for change frequency
/// - Test coverage data
/// - Profiling data for execution frequency
#[derive(Debug)]
pub struct CodeMetricsCollector {
    /// Git history data
    git_history: Option<GitHistory>,
    /// Path to test coverage data
    test_coverage_path: Option<PathBuf>,
    /// Coverage data (loaded from file)
    coverage_data: Option<CoverageData>,
    /// Profiling data
    profiling_data: Option<ProfilingData>,
    /// Cache of computed metrics
    metrics_cache: HashMap<Text, EnhancedCodeMetrics>,
    /// Analysis duration tracking
    total_analysis_time: std::time::Duration,
}

impl CodeMetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        Self {
            git_history: None,
            test_coverage_path: None,
            coverage_data: None,
            profiling_data: None,
            metrics_cache: HashMap::new(),
            total_analysis_time: std::time::Duration::ZERO,
        }
    }

    /// Load git history from repository
    pub fn load_git_history(&mut self, repo_path: &Path) -> MetricsResult<()> {
        self.git_history = Some(GitHistory::load_from_repo(repo_path)?);
        Ok(())
    }

    /// Load test coverage data (lcov format)
    pub fn load_coverage(&mut self, path: &Path) -> MetricsResult<()> {
        self.test_coverage_path = Some(path.to_path_buf());
        self.coverage_data = Some(CoverageData::load_lcov(path)?);
        Ok(())
    }

    /// Load profiling data (perf/callgrind format)
    pub fn load_profiling(&mut self, path: &Path) -> MetricsResult<()> {
        // Try callgrind format first
        let profiling = CoverageData::load_lcov(path)
            .map(|_| ProfilingData::load_callgrind(path))
            .unwrap_or_else(|_| ProfilingData::load_perf(path))?;

        self.profiling_data = Some(profiling);
        Ok(())
    }

    /// Analyze a function and collect metrics
    pub fn analyze_function(&mut self, func: &FunctionDecl) -> EnhancedCodeMetrics {
        let start = Instant::now();
        let func_name = func.name.as_str();

        // Check cache
        if let Some(cached) = self.metrics_cache.get(func_name) {
            return cached.clone();
        }

        // Create visitor and analyze AST
        let mut visitor = MetricsVisitor::new(func_name);
        visitor.visit_function(func);
        let mut metrics = visitor.finish();

        // Enrich with external data
        self.enrich_with_coverage(&mut metrics, func_name);
        self.enrich_with_profiling(&mut metrics, func_name);
        self.enrich_with_git_history(&mut metrics, func_name);

        // Calculate final proof complexity
        metrics.proof_complexity = self.calculate_final_proof_complexity(&metrics);

        // Cache result
        self.metrics_cache
            .insert(func_name.to_text(), metrics.clone());
        self.total_analysis_time += start.elapsed();

        metrics
    }

    /// Analyze all functions in a module
    pub fn analyze_module(&mut self, module: &Module) -> List<EnhancedCodeMetrics> {
        let mut all_metrics = List::new();

        for item in module.items.iter() {
            if let ItemKind::Function(func) = &item.kind {
                let metrics = self.analyze_function(func);
                all_metrics.push(metrics);
            }
        }

        all_metrics
    }

    /// Calculate change frequency from git history
    pub fn calculate_change_frequency(&self, file: &Path) -> f64 {
        if let Some(history) = &self.git_history {
            // Calculate over 12 weeks (3 months)
            history.change_frequency(file, 12.0)
        } else {
            // Default: assume moderately stable
            0.5
        }
    }

    /// Enrich metrics with coverage data
    fn enrich_with_coverage(&self, metrics: &mut EnhancedCodeMetrics, func_name: &str) {
        if let Some(coverage) = &self.coverage_data {
            metrics.test_coverage = coverage.function_coverage(func_name);
        }
    }

    /// Enrich metrics with profiling data
    fn enrich_with_profiling(&self, metrics: &mut EnhancedCodeMetrics, func_name: &str) {
        if let Some(profiling) = &self.profiling_data {
            metrics.execution_frequency = profiling.execution_frequency(func_name);

            // Adjust criticality based on execution frequency
            if profiling.is_hot_path(func_name) {
                metrics.criticality_score = metrics.criticality_score.max(7);
            }
        }
    }

    /// Enrich metrics with git history
    fn enrich_with_git_history(&self, metrics: &mut EnhancedCodeMetrics, _func_name: &str) {
        if let Maybe::Some(ref file_path) = metrics.file_path {
            let path = PathBuf::from(file_path.as_str());
            metrics.change_frequency_per_week = self.calculate_change_frequency(&path);
        }
    }

    /// Calculate final proof complexity considering all factors
    fn calculate_final_proof_complexity(&self, metrics: &EnhancedCodeMetrics) -> u32 {
        let mut complexity = metrics.proof_complexity;

        // Adjust based on test coverage (better coverage = easier to verify)
        if metrics.test_coverage > 0.9 {
            complexity = (complexity as f64 * 0.8) as u32;
        }

        // Adjust based on change frequency (unstable code is harder to verify)
        if metrics.change_frequency_per_week > 1.0 {
            complexity = (complexity as f64 * 1.2) as u32;
        }

        complexity
    }

    /// Get total analysis time
    pub fn total_analysis_time(&self) -> std::time::Duration {
        self.total_analysis_time
    }

    /// Get metrics from cache
    pub fn get_cached_metrics(&self, func_name: &str) -> Option<&EnhancedCodeMetrics> {
        self.metrics_cache.get(func_name)
    }

    /// Clear metrics cache
    pub fn clear_cache(&mut self) {
        self.metrics_cache.clear();
    }

    /// Convert enhanced metrics to basic CodeMetrics for transition analysis
    pub fn to_code_metrics(&self, func_name: &str) -> CodeMetrics {
        self.metrics_cache
            .get(func_name)
            .map(|m| m.to_code_metrics())
            .unwrap_or_default()
    }
}

impl Default for CodeMetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Public API Functions
// =============================================================================

/// Analyze a function and return enhanced metrics
///
/// Convenience function for quick analysis without collector setup.
pub fn analyze_function(func: &FunctionDecl) -> EnhancedCodeMetrics {
    let mut collector = CodeMetricsCollector::new();
    collector.analyze_function(func)
}

/// Analyze a module and return metrics for all functions
pub fn analyze_module(module: &Module) -> List<EnhancedCodeMetrics> {
    let mut collector = CodeMetricsCollector::new();
    collector.analyze_module(module)
}

/// Calculate cyclomatic complexity from function CFG
pub fn complexity_from_cfg(cfg: &EscapeCFG) -> u32 {
    calculate_cyclomatic_complexity(cfg)
}

/// Calculate loop nesting depth from function CFG
pub fn nesting_from_cfg(cfg: &EscapeCFG) -> u32 {
    analyze_loop_nesting(cfg)
}

// =============================================================================
// Tests Module Notice
// =============================================================================

// Tests are in tests/metrics_tests.rs per CLAUDE.md standards
