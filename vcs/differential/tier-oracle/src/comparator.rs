//! Comparator - Compare outputs across tiers
//!
//! This module provides comprehensive comparison of execution outputs
//! between different tiers. It handles:
//!
//! - Stdout comparison with normalization
//! - Stderr comparison
//! - Exit code comparison
//! - Float precision tolerance
//! - Collection ordering tolerance
//! - Async output reordering

use std::collections::{BTreeMap, HashSet};
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::executor::ExecutionResult;
use crate::Tier;

/// Configuration for the comparator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparatorConfig {
    /// Whether to normalize outputs before comparison
    pub normalize: bool,
    /// Float comparison epsilon
    pub float_epsilon: f64,
    /// Whether to allow unordered collection output
    pub allow_unordered_collections: bool,
    /// Whether to allow async output reordering
    pub allow_async_reordering: bool,
    /// Number of context lines to include in diffs
    pub context_lines: usize,
    /// Whether to ignore whitespace differences
    pub ignore_whitespace: bool,
    /// Whether to strip memory addresses
    pub strip_addresses: bool,
    /// Whether to strip timestamps
    pub strip_timestamps: bool,
    /// Whether to strip ANSI color codes
    pub strip_ansi: bool,
    /// Custom patterns to strip
    pub strip_patterns: Vec<String>,
    /// Maximum allowed execution time ratio
    pub max_time_ratio: f64,
}

impl Default for ComparatorConfig {
    fn default() -> Self {
        Self {
            normalize: true,
            float_epsilon: 1e-10,
            allow_unordered_collections: true,
            allow_async_reordering: false,
            context_lines: 3,
            ignore_whitespace: false,
            strip_addresses: true,
            strip_timestamps: true,
            strip_ansi: true,
            strip_patterns: Vec::new(),
            max_time_ratio: 100.0,
        }
    }
}

/// Result of comparing two execution results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    /// Whether the outputs are equivalent
    pub equivalent: bool,
    /// List of behavioral differences found
    pub differences: Vec<BehaviorDiff>,
    /// Unified diff of stdout
    pub stdout_diff: Option<String>,
    /// Unified diff of stderr
    pub stderr_diff: Option<String>,
    /// Performance comparison
    pub performance: PerformanceComparison,
}

impl ComparisonResult {
    /// Create a result indicating equivalence
    pub fn equivalent() -> Self {
        Self {
            equivalent: true,
            differences: Vec::new(),
            stdout_diff: None,
            stderr_diff: None,
            performance: PerformanceComparison::default(),
        }
    }

    /// Create a result indicating differences
    pub fn different(differences: Vec<BehaviorDiff>) -> Self {
        Self {
            equivalent: false,
            differences,
            stdout_diff: None,
            stderr_diff: None,
            performance: PerformanceComparison::default(),
        }
    }

    /// Add stdout diff
    pub fn with_stdout_diff(mut self, diff: String) -> Self {
        self.stdout_diff = Some(diff);
        self
    }

    /// Add stderr diff
    pub fn with_stderr_diff(mut self, diff: String) -> Self {
        self.stderr_diff = Some(diff);
        self
    }

    /// Add performance comparison
    pub fn with_performance(mut self, perf: PerformanceComparison) -> Self {
        self.performance = perf;
        self
    }
}

/// A specific behavioral difference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorDiff {
    /// Kind of difference
    pub kind: String,
    /// Summary of the difference
    pub summary: String,
    /// Detailed information
    pub details: Vec<DiffDetail>,
    /// Suggested fix (if any)
    pub suggested_fix: Option<String>,
    /// Severity level
    pub severity: DiffSeverity,
}

/// Detailed information about a difference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffDetail {
    /// Location (line number, position, etc.)
    pub location: String,
    /// Expected value
    pub expected: String,
    /// Actual value
    pub actual: String,
    /// Context around the difference
    pub context: Vec<String>,
}

/// Severity of a difference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffSeverity {
    /// Informational only (e.g., performance difference)
    Info,
    /// Warning (e.g., float precision, ordering)
    Warning,
    /// Error (e.g., different output, crash)
    Error,
    /// Critical (e.g., crash vs success)
    Critical,
}

/// Performance comparison between tiers
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerformanceComparison {
    /// Duration ratio (tier2 / tier1)
    pub time_ratio: f64,
    /// Whether performance differs significantly
    pub significant_difference: bool,
    /// Tier 1 duration in ms
    pub tier1_ms: u64,
    /// Tier 2 duration in ms
    pub tier2_ms: u64,
    /// Memory ratio (if available)
    pub memory_ratio: Option<f64>,
}

/// Main comparator
pub struct Comparator {
    config: ComparatorConfig,
    compiled_patterns: Vec<Regex>,
}

impl Comparator {
    /// Create a new comparator with the given configuration
    pub fn new(config: ComparatorConfig) -> Self {
        let compiled_patterns: Vec<Regex> = config
            .strip_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();

        Self {
            config,
            compiled_patterns,
        }
    }

    /// Compare two execution results
    pub fn compare(
        &self,
        expected: &ExecutionResult,
        actual: &ExecutionResult,
    ) -> Result<ComparisonResult> {
        let mut differences = Vec::new();

        // Check for crashes
        if expected.crashed != actual.crashed {
            differences.push(self.crash_difference(expected, actual));
            return Ok(ComparisonResult::different(differences));
        }

        // Check for timeouts
        if expected.timed_out != actual.timed_out {
            differences.push(self.timeout_difference(expected, actual));
            return Ok(ComparisonResult::different(differences));
        }

        // Compare exit codes
        if expected.exit_code != actual.exit_code {
            differences.push(self.exit_code_difference(expected, actual));
        }

        // Normalize and compare stdout
        let expected_stdout = self.normalize(&expected.stdout);
        let actual_stdout = self.normalize(&actual.stdout);

        if expected_stdout != actual_stdout {
            if let Some(diff) = self.compare_outputs(&expected_stdout, &actual_stdout) {
                differences.push(diff);
            }
        }

        // Compare stderr (less strict)
        let expected_stderr = self.normalize(&expected.stderr);
        let actual_stderr = self.normalize(&actual.stderr);

        if expected_stderr != actual_stderr {
            // Stderr differences are warnings, not errors
            let mut diff = self.create_output_diff(
                "stderr",
                &expected_stderr,
                &actual_stderr,
            );
            diff.severity = DiffSeverity::Warning;
            differences.push(diff);
        }

        // Performance comparison
        let performance = self.compare_performance(expected, actual);

        if differences.is_empty() {
            let mut result = ComparisonResult::equivalent();
            result.performance = performance;
            Ok(result)
        } else {
            let stdout_diff = self.generate_unified_diff(&expected_stdout, &actual_stdout);
            let stderr_diff = self.generate_unified_diff(&expected_stderr, &actual_stderr);

            let result = ComparisonResult::different(differences)
                .with_stdout_diff(stdout_diff)
                .with_stderr_diff(stderr_diff)
                .with_performance(performance);

            Ok(result)
        }
    }

    /// Normalize an output string
    fn normalize(&self, input: &str) -> String {
        if !self.config.normalize {
            return input.to_string();
        }

        let mut output = input.to_string();

        // Normalize line endings
        output = output.replace("\r\n", "\n").replace('\r', "\n");

        // Strip ANSI codes
        if self.config.strip_ansi {
            let ansi_pattern = Regex::new(r"\x1B\[[0-9;]*[a-zA-Z]").unwrap();
            output = ansi_pattern.replace_all(&output, "").to_string();
        }

        // Strip memory addresses
        if self.config.strip_addresses {
            let addr_pattern = Regex::new(r"0x[0-9a-fA-F]{6,16}").unwrap();
            output = addr_pattern.replace_all(&output, "<ADDR>").to_string();
        }

        // Strip timestamps
        if self.config.strip_timestamps {
            // ISO 8601
            let iso_pattern = Regex::new(
                r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})?"
            ).unwrap();
            output = iso_pattern.replace_all(&output, "<TIME>").to_string();

            // Common time formats
            let time_pattern = Regex::new(r"\b\d{2}:\d{2}:\d{2}(\.\d{1,6})?\b").unwrap();
            output = time_pattern.replace_all(&output, "<TIME>").to_string();
        }

        // Apply custom patterns
        for pattern in &self.compiled_patterns {
            output = pattern.replace_all(&output, "<STRIPPED>").to_string();
        }

        // Normalize floats if needed
        if self.config.float_epsilon > 0.0 {
            output = self.normalize_floats(&output);
        }

        // Ignore whitespace
        if self.config.ignore_whitespace {
            output = output
                .lines()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join("\n");
        }

        // Trim trailing whitespace from lines
        output = output
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        output
    }

    /// Normalize floating-point numbers for comparison
    fn normalize_floats(&self, input: &str) -> String {
        let float_pattern = Regex::new(r"-?\d+\.\d+([eE][+-]?\d+)?").unwrap();

        float_pattern
            .replace_all(input, |caps: &regex::Captures| {
                let float_str = &caps[0];
                if let Ok(f) = float_str.parse::<f64>() {
                    if f.is_nan() {
                        return "NaN".to_string();
                    }
                    if f.is_infinite() {
                        return if f > 0.0 { "Inf" } else { "-Inf" }.to_string();
                    }
                    // Round to avoid precision differences
                    format!("{:.10}", f)
                } else {
                    float_str.to_string()
                }
            })
            .to_string()
    }

    /// Compare two output strings
    fn compare_outputs(&self, expected: &str, actual: &str) -> Option<BehaviorDiff> {
        // Try semantic comparison first
        if self.config.allow_unordered_collections {
            if let Some(diff) = self.semantic_compare(expected, actual) {
                return Some(diff);
            } else {
                return None; // Semantically equivalent
            }
        }

        // If unordered not allowed, do direct comparison
        if expected == actual {
            return None;
        }

        Some(self.create_output_diff("stdout", expected, actual))
    }

    /// Semantic comparison with tolerance for ordering
    fn semantic_compare(&self, expected: &str, actual: &str) -> Option<BehaviorDiff> {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        // First check exact match
        if expected_lines == actual_lines {
            return None;
        }

        // Check if lines are the same but reordered (within marked regions)
        if self.check_unordered_equivalent(&expected_lines, &actual_lines) {
            return None;
        }

        // Check float equivalence
        if self.check_float_equivalent(expected, actual) {
            return None;
        }

        // Real difference found
        Some(self.create_output_diff("stdout", expected, actual))
    }

    /// Check if two outputs are equivalent when ignoring order
    fn check_unordered_equivalent(&self, expected: &[&str], actual: &[&str]) -> bool {
        if expected.len() != actual.len() {
            return false;
        }

        // Check if they contain the same lines (multiset equality)
        let mut expected_set: BTreeMap<&str, usize> = BTreeMap::new();
        let mut actual_set: BTreeMap<&str, usize> = BTreeMap::new();

        for line in expected {
            *expected_set.entry(*line).or_insert(0) += 1;
        }

        for line in actual {
            *actual_set.entry(*line).or_insert(0) += 1;
        }

        expected_set == actual_set
    }

    /// Check if two outputs are equivalent considering float epsilon
    fn check_float_equivalent(&self, expected: &str, actual: &str) -> bool {
        let float_pattern = Regex::new(r"-?\d+\.\d+([eE][+-]?\d+)?").unwrap();

        let expected_floats: Vec<f64> = float_pattern
            .find_iter(expected)
            .filter_map(|m| m.as_str().parse().ok())
            .collect();

        let actual_floats: Vec<f64> = float_pattern
            .find_iter(actual)
            .filter_map(|m| m.as_str().parse().ok())
            .collect();

        if expected_floats.len() != actual_floats.len() {
            return false;
        }

        for (e, a) in expected_floats.iter().zip(actual_floats.iter()) {
            if !self.floats_equal(*e, *a) {
                return false;
            }
        }

        // Also check non-float content
        let expected_no_floats = float_pattern.replace_all(expected, "FLOAT");
        let actual_no_floats = float_pattern.replace_all(actual, "FLOAT");

        expected_no_floats == actual_no_floats
    }

    /// Compare two floats with epsilon
    fn floats_equal(&self, a: f64, b: f64) -> bool {
        if a.is_nan() && b.is_nan() {
            return true;
        }
        if a.is_infinite() && b.is_infinite() {
            return a.signum() == b.signum();
        }
        (a - b).abs() < self.config.float_epsilon
    }

    /// Create a diff for crash difference
    fn crash_difference(&self, expected: &ExecutionResult, actual: &ExecutionResult) -> BehaviorDiff {
        let (crashed_tier, normal_tier) = if actual.crashed {
            (actual.tier, expected.tier)
        } else {
            (expected.tier, actual.tier)
        };

        BehaviorDiff {
            kind: "crash".to_string(),
            summary: format!("{} crashed while {} completed normally", crashed_tier, normal_tier),
            details: vec![DiffDetail {
                location: "execution".to_string(),
                expected: if expected.crashed { "crash".to_string() } else { "success".to_string() },
                actual: if actual.crashed { "crash".to_string() } else { "success".to_string() },
                context: vec![
                    if actual.crashed {
                        format!("Signal: {:?}", actual.signal)
                    } else {
                        format!("Signal: {:?}", expected.signal)
                    },
                ],
            }],
            suggested_fix: Some("Check for undefined behavior or memory issues".to_string()),
            severity: DiffSeverity::Critical,
        }
    }

    /// Create a diff for timeout difference
    fn timeout_difference(&self, expected: &ExecutionResult, actual: &ExecutionResult) -> BehaviorDiff {
        let (timed_out_tier, normal_tier) = if actual.timed_out {
            (actual.tier, expected.tier)
        } else {
            (expected.tier, actual.tier)
        };

        BehaviorDiff {
            kind: "timeout".to_string(),
            summary: format!("{} timed out while {} completed", timed_out_tier, normal_tier),
            details: vec![DiffDetail {
                location: "execution".to_string(),
                expected: if expected.timed_out {
                    "timeout".to_string()
                } else {
                    format!("completed in {}ms", expected.duration_ms)
                },
                actual: if actual.timed_out {
                    "timeout".to_string()
                } else {
                    format!("completed in {}ms", actual.duration_ms)
                },
                context: vec![],
            }],
            suggested_fix: Some("Check for infinite loops or performance issues".to_string()),
            severity: DiffSeverity::Critical,
        }
    }

    /// Create a diff for exit code difference
    fn exit_code_difference(&self, expected: &ExecutionResult, actual: &ExecutionResult) -> BehaviorDiff {
        BehaviorDiff {
            kind: "exit_code".to_string(),
            summary: format!(
                "Exit code differs: {:?} vs {:?}",
                expected.exit_code, actual.exit_code
            ),
            details: vec![DiffDetail {
                location: "exit".to_string(),
                expected: format!("{:?}", expected.exit_code),
                actual: format!("{:?}", actual.exit_code),
                context: vec![
                    format!("{} stderr: {}", expected.tier, truncate(&expected.stderr, 100)),
                    format!("{} stderr: {}", actual.tier, truncate(&actual.stderr, 100)),
                ],
            }],
            suggested_fix: None,
            severity: DiffSeverity::Error,
        }
    }

    /// Create an output diff
    fn create_output_diff(&self, kind: &str, expected: &str, actual: &str) -> BehaviorDiff {
        let diff_lines = self.find_diff_locations(expected, actual);

        let details: Vec<DiffDetail> = diff_lines
            .iter()
            .take(5) // Limit to first 5 differences
            .map(|(line_num, exp, act)| DiffDetail {
                location: format!("line {}", line_num),
                expected: truncate(exp, 200),
                actual: truncate(act, 200),
                context: self.get_context(expected, *line_num),
            })
            .collect();

        BehaviorDiff {
            kind: kind.to_string(),
            summary: format!(
                "Output differs at {} location(s)",
                diff_lines.len()
            ),
            details,
            suggested_fix: None,
            severity: DiffSeverity::Error,
        }
    }

    /// Find locations where outputs differ
    fn find_diff_locations(&self, expected: &str, actual: &str) -> Vec<(usize, String, String)> {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        let mut diffs = Vec::new();

        let max_len = expected_lines.len().max(actual_lines.len());
        for i in 0..max_len {
            let exp = expected_lines.get(i).unwrap_or(&"<missing>");
            let act = actual_lines.get(i).unwrap_or(&"<missing>");

            if exp != act {
                diffs.push((i + 1, exp.to_string(), act.to_string()));
            }
        }

        diffs
    }

    /// Get context lines around a position
    fn get_context(&self, output: &str, line_num: usize) -> Vec<String> {
        let lines: Vec<&str> = output.lines().collect();
        let mut context = Vec::new();

        let start = line_num.saturating_sub(self.config.context_lines + 1);
        let end = (line_num + self.config.context_lines).min(lines.len());

        for i in start..end {
            if let Some(line) = lines.get(i) {
                let prefix = if i + 1 == line_num { "> " } else { "  " };
                context.push(format!("{}{}: {}", prefix, i + 1, line));
            }
        }

        context
    }

    /// Generate unified diff
    fn generate_unified_diff(&self, expected: &str, actual: &str) -> String {
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        let mut diff = String::new();
        diff.push_str("--- Expected (Reference Tier)\n");
        diff.push_str("+++ Actual (Comparison Tier)\n");

        let max_len = expected_lines.len().max(actual_lines.len());
        for i in 0..max_len {
            let exp = expected_lines.get(i);
            let act = actual_lines.get(i);

            match (exp, act) {
                (Some(e), Some(a)) if e == a => {
                    diff.push_str(&format!(" {}\n", e));
                }
                (Some(e), Some(a)) => {
                    diff.push_str(&format!("-{}\n", e));
                    diff.push_str(&format!("+{}\n", a));
                }
                (Some(e), None) => {
                    diff.push_str(&format!("-{}\n", e));
                }
                (None, Some(a)) => {
                    diff.push_str(&format!("+{}\n", a));
                }
                (None, None) => {}
            }
        }

        diff
    }

    /// Compare performance between tiers
    fn compare_performance(
        &self,
        expected: &ExecutionResult,
        actual: &ExecutionResult,
    ) -> PerformanceComparison {
        let tier1_ms = expected.duration_ms;
        let tier2_ms = actual.duration_ms;

        let time_ratio = if tier1_ms > 0 {
            tier2_ms as f64 / tier1_ms as f64
        } else {
            1.0
        };

        let significant_difference = time_ratio > self.config.max_time_ratio
            || time_ratio < (1.0 / self.config.max_time_ratio);

        let memory_ratio = match (expected.peak_memory, actual.peak_memory) {
            (Some(m1), Some(m2)) if m1 > 0 => Some(m2 as f64 / m1 as f64),
            _ => None,
        };

        PerformanceComparison {
            time_ratio,
            significant_difference,
            tier1_ms,
            tier2_ms,
            memory_ratio,
        }
    }
}

/// Helper to truncate strings
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Quick comparison helper
pub fn quick_compare(expected: &ExecutionResult, actual: &ExecutionResult) -> bool {
    expected.exit_code == actual.exit_code
        && expected.stdout == actual.stdout
        && expected.stderr == actual.stderr
}

/// Semantic comparison helper
pub fn semantic_compare(
    expected: &ExecutionResult,
    actual: &ExecutionResult,
    float_epsilon: f64,
) -> bool {
    let comparator = Comparator::new(ComparatorConfig {
        float_epsilon,
        ..Default::default()
    });

    comparator
        .compare(expected, actual)
        .map(|r| r.equivalent)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_result(tier: Tier, stdout: &str, exit_code: i32) -> ExecutionResult {
        ExecutionResult {
            tier,
            success: exit_code == 0,
            exit_code: Some(exit_code),
            stdout: stdout.to_string(),
            stderr: String::new(),
            duration: Duration::from_millis(100),
            duration_ms: 100,
            timed_out: false,
            crashed: false,
            signal: None,
            peak_memory: None,
        }
    }

    #[test]
    fn test_exact_match() {
        let comparator = Comparator::new(ComparatorConfig::default());
        let r1 = make_result(Tier::Interpreter, "hello world", 0);
        let r2 = make_result(Tier::Aot, "hello world", 0);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(result.equivalent);
    }

    #[test]
    fn test_different_output() {
        let comparator = Comparator::new(ComparatorConfig::default());
        let r1 = make_result(Tier::Interpreter, "hello", 0);
        let r2 = make_result(Tier::Aot, "world", 0);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(!result.equivalent);
        assert!(!result.differences.is_empty());
    }

    #[test]
    fn test_float_epsilon() {
        let comparator = Comparator::new(ComparatorConfig {
            float_epsilon: 1e-6,
            ..Default::default()
        });

        let r1 = make_result(Tier::Interpreter, "result: 3.14159265", 0);
        let r2 = make_result(Tier::Aot, "result: 3.14159266", 0);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(result.equivalent);
    }

    #[test]
    fn test_address_normalization() {
        let comparator = Comparator::new(ComparatorConfig {
            strip_addresses: true,
            ..Default::default()
        });

        let r1 = make_result(Tier::Interpreter, "Object at 0x7fff1234abcd", 0);
        let r2 = make_result(Tier::Aot, "Object at 0x7fff9876fedc", 0);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(result.equivalent);
    }

    #[test]
    fn test_exit_code_difference() {
        let comparator = Comparator::new(ComparatorConfig::default());
        let r1 = make_result(Tier::Interpreter, "done", 0);
        let r2 = make_result(Tier::Aot, "done", 1);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(!result.equivalent);
        assert!(result.differences.iter().any(|d| d.kind == "exit_code"));
    }

    #[test]
    fn test_unordered_comparison() {
        let comparator = Comparator::new(ComparatorConfig {
            allow_unordered_collections: true,
            ..Default::default()
        });

        let r1 = make_result(Tier::Interpreter, "a\nb\nc", 0);
        let r2 = make_result(Tier::Aot, "c\na\nb", 0);

        let result = comparator.compare(&r1, &r2).unwrap();
        assert!(result.equivalent);
    }

    #[test]
    fn test_quick_compare() {
        let r1 = make_result(Tier::Interpreter, "hello", 0);
        let r2 = make_result(Tier::Aot, "hello", 0);

        assert!(quick_compare(&r1, &r2));

        let r3 = make_result(Tier::Aot, "world", 0);
        assert!(!quick_compare(&r1, &r3));
    }
}
