//! Semantic equivalence checking for differential testing
//!
//! This module provides sophisticated comparison algorithms that go beyond
//! simple string matching to determine if two outputs are semantically equivalent.
//!
//! Key features:
//! - Floating-point tolerance with configurable epsilon
//! - Unordered collection comparison (sets, maps)
//! - Structural equivalence for nested data
//! - Error message semantic matching
//! - Timing-independent async output comparison

use crate::normalizer::{NormalizationConfig, NormalizedValue, Normalizer};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;

/// Configuration for semantic equivalence checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivalenceConfig {
    /// Epsilon for floating-point comparison
    pub float_epsilon: f64,
    /// Whether to use relative epsilon for large floats
    pub use_relative_epsilon: bool,
    /// Relative epsilon factor (e.g., 1e-10 means 10^-10 relative difference)
    pub relative_epsilon: f64,
    /// ULP (Units in Last Place) tolerance for float comparison
    pub ulp_tolerance: u64,
    /// Whether to use ULP-based comparison
    pub use_ulp_comparison: bool,
    /// How to handle NaN values
    pub nan_handling: NaNHandling,
    /// Whether to treat denormalized numbers as zero
    pub flush_denormals_to_zero: bool,
    /// Whether unordered collections (sets, maps) can differ in order
    pub allow_unordered_collections: bool,
    /// Whether async output lines can be reordered
    pub allow_async_reordering: bool,
    /// Patterns that mark async regions (lines can be reordered within)
    pub async_region_markers: Vec<String>,
    /// Whether to compare error messages semantically
    pub semantic_error_matching: bool,
    /// Error code patterns to extract
    pub error_code_patterns: Vec<String>,
    /// Whether whitespace differences are acceptable
    pub ignore_whitespace: bool,
    /// Whether empty line differences are acceptable
    pub ignore_empty_lines: bool,
    /// Custom equivalence rules
    pub custom_rules: Vec<EquivalenceRule>,
}

/// How to handle NaN values in comparison
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NaNHandling {
    /// Treat all NaN values as equal (NaN == NaN)
    TreatAsEqual,
    /// NaN is never equal to anything (IEEE 754 strict)
    Ieee754Strict,
    /// Require same NaN bit pattern (signaling vs quiet, payload)
    ExactBitPattern,
}

impl Default for EquivalenceConfig {
    fn default() -> Self {
        Self {
            float_epsilon: 1e-10,
            use_relative_epsilon: true,
            relative_epsilon: 1e-10,
            ulp_tolerance: 4,
            use_ulp_comparison: true,
            nan_handling: NaNHandling::TreatAsEqual,
            flush_denormals_to_zero: true,
            allow_unordered_collections: true,
            allow_async_reordering: false,
            async_region_markers: vec!["@async:".to_string()],
            semantic_error_matching: true,
            error_code_patterns: vec![r"E\d{4}".to_string(), r"error\[\w+\]".to_string()],
            ignore_whitespace: false,
            ignore_empty_lines: false,
            custom_rules: vec![],
        }
    }
}

impl EquivalenceConfig {
    /// Create a strict configuration (minimal tolerance)
    pub fn strict() -> Self {
        Self {
            float_epsilon: 0.0,
            use_relative_epsilon: false,
            relative_epsilon: 0.0,
            ulp_tolerance: 0,
            use_ulp_comparison: false,
            nan_handling: NaNHandling::Ieee754Strict,
            flush_denormals_to_zero: false,
            allow_unordered_collections: false,
            allow_async_reordering: false,
            async_region_markers: vec![],
            semantic_error_matching: false,
            error_code_patterns: vec![],
            ignore_whitespace: false,
            ignore_empty_lines: false,
            custom_rules: vec![],
        }
    }

    /// Create a lenient configuration (maximum tolerance)
    pub fn lenient() -> Self {
        Self {
            float_epsilon: 1e-6,
            use_relative_epsilon: true,
            relative_epsilon: 1e-6,
            ulp_tolerance: 16,
            use_ulp_comparison: true,
            nan_handling: NaNHandling::TreatAsEqual,
            flush_denormals_to_zero: true,
            allow_unordered_collections: true,
            allow_async_reordering: true,
            async_region_markers: vec![
                "@async:".to_string(),
                "@concurrent:".to_string(),
                "@parallel:".to_string(),
            ],
            semantic_error_matching: true,
            error_code_patterns: vec![r"E\d{4}".to_string(), r"error\[\w+\]".to_string()],
            ignore_whitespace: true,
            ignore_empty_lines: true,
            custom_rules: vec![],
        }
    }

    /// Create a configuration optimized for numeric computation comparison
    pub fn numeric() -> Self {
        Self {
            float_epsilon: 1e-12,
            use_relative_epsilon: true,
            relative_epsilon: 1e-12,
            ulp_tolerance: 4,
            use_ulp_comparison: true,
            nan_handling: NaNHandling::TreatAsEqual,
            flush_denormals_to_zero: true,
            allow_unordered_collections: false,
            allow_async_reordering: false,
            async_region_markers: vec![],
            semantic_error_matching: false,
            error_code_patterns: vec![],
            ignore_whitespace: false,
            ignore_empty_lines: false,
            custom_rules: vec![],
        }
    }
}

/// Custom equivalence rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivalenceRule {
    /// Name of the rule
    pub name: String,
    /// Pattern to match
    pub pattern: String,
    /// Transformation to apply
    pub transform: TransformType,
}

/// Type of transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransformType {
    /// Strip matching text
    Strip,
    /// Replace with constant
    Replace(String),
    /// Normalize using custom regex
    Normalize { from: String, to: String },
    /// Sort lines matching pattern
    SortLines,
}

/// Result of equivalence check
#[derive(Debug, Clone, PartialEq)]
pub enum EquivalenceResult {
    /// Outputs are equivalent
    Equivalent,
    /// Outputs differ
    Different(Vec<Difference>),
}

impl EquivalenceResult {
    pub fn is_equivalent(&self) -> bool {
        matches!(self, EquivalenceResult::Equivalent)
    }

    pub fn differences(&self) -> Option<&Vec<Difference>> {
        match self {
            EquivalenceResult::Different(diffs) => Some(diffs),
            EquivalenceResult::Equivalent => None,
        }
    }
}

/// A single difference between outputs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Difference {
    /// Location of the difference
    pub location: DiffLocation,
    /// Type of difference
    pub kind: DiffKind,
    /// Value from first output
    pub expected: String,
    /// Value from second output
    pub actual: String,
    /// Severity of the difference
    pub severity: DiffSeverity,
}

/// Location of a difference
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DiffLocation {
    /// Line number (1-indexed)
    Line(usize),
    /// Line range
    LineRange { start: usize, end: usize },
    /// Path in structured data
    Path(Vec<String>),
    /// Entire output
    Global,
}

impl std::fmt::Display for DiffLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffLocation::Line(n) => write!(f, "line {}", n),
            DiffLocation::LineRange { start, end } => write!(f, "lines {}-{}", start, end),
            DiffLocation::Path(path) => write!(f, "at {}", path.join(".")),
            DiffLocation::Global => write!(f, "global"),
        }
    }
}

/// Kind of difference
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DiffKind {
    /// Values differ
    ValueMismatch,
    /// Float values differ beyond tolerance
    FloatMismatch {
        expected: f64,
        actual: f64,
        difference: f64,
    },
    /// Type mismatch
    TypeMismatch {
        expected_type: String,
        actual_type: String,
    },
    /// Missing in actual
    Missing,
    /// Extra in actual
    Extra,
    /// Order differs (for ordered collections)
    OrderMismatch,
    /// Length differs
    LengthMismatch { expected: usize, actual: usize },
    /// Line content differs
    LineDiff,
}

/// Severity of difference
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DiffSeverity {
    /// Informational only
    Info,
    /// Warning (may be acceptable)
    Warning,
    /// Error (semantic difference)
    Error,
    /// Critical (likely bug)
    Critical,
}

/// Semantic equivalence checker
pub struct SemanticEquivalenceChecker {
    config: EquivalenceConfig,
    normalizer: Normalizer,
}

impl SemanticEquivalenceChecker {
    /// Create a new checker with the given configuration
    pub fn new(config: EquivalenceConfig) -> Self {
        Self {
            config,
            normalizer: Normalizer::new(NormalizationConfig::semantic()),
        }
    }

    /// Create a checker with default configuration
    pub fn default_config() -> Self {
        Self::new(EquivalenceConfig::default())
    }

    /// Check if two outputs are semantically equivalent
    pub fn check(&self, expected: &str, actual: &str) -> EquivalenceResult {
        // Step 1: Normalize both outputs
        let norm_expected = self.normalizer.normalize(expected);
        let norm_actual = self.normalizer.normalize(actual);

        // Step 2: Quick exact match check
        if norm_expected == norm_actual {
            return EquivalenceResult::Equivalent;
        }

        // Step 3: Pre-process for whitespace/empty lines if configured
        let (proc_expected, proc_actual) =
            if self.config.ignore_whitespace || self.config.ignore_empty_lines {
                (
                    self.preprocess(&norm_expected),
                    self.preprocess(&norm_actual),
                )
            } else {
                (norm_expected.clone(), norm_actual.clone())
            };

        if proc_expected == proc_actual {
            return EquivalenceResult::Equivalent;
        }

        // Step 4: Try structured comparison
        let expected_value = NormalizedValue::parse(&proc_expected);
        let actual_value = NormalizedValue::parse(&proc_actual);

        if self.check_value_equivalence(&expected_value, &actual_value, &[]) {
            return EquivalenceResult::Equivalent;
        }

        // Step 5: Line-by-line comparison
        let differences = self.compare_lines(&proc_expected, &proc_actual);

        if differences.is_empty() {
            EquivalenceResult::Equivalent
        } else {
            EquivalenceResult::Different(differences)
        }
    }

    /// Pre-process output according to config
    fn preprocess(&self, input: &str) -> String {
        let mut lines: Vec<&str> = input.lines().collect();

        if self.config.ignore_empty_lines {
            lines.retain(|line| !line.trim().is_empty());
        }

        if self.config.ignore_whitespace {
            lines
                .iter()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            lines.join("\n")
        }
    }

    /// Check value equivalence recursively
    fn check_value_equivalence(
        &self,
        expected: &NormalizedValue,
        actual: &NormalizedValue,
        path: &[String],
    ) -> bool {
        match (expected, actual) {
            // Null comparison
            (NormalizedValue::Null, NormalizedValue::Null) => true,

            // Bool comparison
            (NormalizedValue::Bool(a), NormalizedValue::Bool(b)) => a == b,

            // Int comparison
            (NormalizedValue::Int(a), NormalizedValue::Int(b)) => a == b,

            // Float comparison with tolerance
            (NormalizedValue::Float { value: a, .. }, NormalizedValue::Float { value: b, .. }) => {
                self.floats_equivalent(*a, *b)
            }

            // Int-Float cross comparison
            (NormalizedValue::Int(a), NormalizedValue::Float { value: b, .. }) => {
                self.floats_equivalent(*a as f64, *b)
            }
            (NormalizedValue::Float { value: a, .. }, NormalizedValue::Int(b)) => {
                self.floats_equivalent(*a, *b as f64)
            }

            // Text comparison
            (NormalizedValue::Text(a), NormalizedValue::Text(b)) => a == b,

            // List comparison
            (NormalizedValue::List(a), NormalizedValue::List(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                a.iter().zip(b.iter()).enumerate().all(|(i, (x, y))| {
                    let mut new_path = path.to_vec();
                    new_path.push(format!("[{}]", i));
                    self.check_value_equivalence(x, y, &new_path)
                })
            }

            // Set comparison (unordered)
            (NormalizedValue::Set(a), NormalizedValue::Set(b)) => {
                if !self.config.allow_unordered_collections {
                    // Require same order
                    a == b
                } else {
                    // Allow any order
                    self.sets_equivalent(a, b)
                }
            }

            // Map comparison
            (NormalizedValue::Map(a), NormalizedValue::Map(b)) => {
                if a.len() != b.len() {
                    return false;
                }
                a.iter().all(|(k, v)| {
                    if let Some(v2) = b.get(k) {
                        let mut new_path = path.to_vec();
                        new_path.push(k.clone());
                        self.check_value_equivalence(v, v2, &new_path)
                    } else {
                        false
                    }
                })
            }

            // Different types
            _ => false,
        }
    }

    /// Check if two floats are equivalent within tolerance
    fn floats_equivalent(&self, a: f64, b: f64) -> bool {
        // Handle denormalized numbers
        let (a, b) = if self.config.flush_denormals_to_zero {
            (flush_denormal(a), flush_denormal(b))
        } else {
            (a, b)
        };

        // Handle NaN based on configuration
        if a.is_nan() || b.is_nan() {
            return match self.config.nan_handling {
                NaNHandling::TreatAsEqual => a.is_nan() && b.is_nan(),
                NaNHandling::Ieee754Strict => false,
                NaNHandling::ExactBitPattern => a.to_bits() == b.to_bits(),
            };
        }

        // Handle infinity
        if a.is_infinite() && b.is_infinite() {
            return a.signum() == b.signum();
        }
        if a.is_infinite() || b.is_infinite() {
            return false;
        }

        // Exact equality check (handles -0.0 == +0.0)
        if a == b {
            return true;
        }

        let diff = (a - b).abs();

        // Absolute epsilon check
        if diff <= self.config.float_epsilon {
            return true;
        }

        // Relative epsilon check
        if self.config.use_relative_epsilon {
            let max_val = a.abs().max(b.abs());
            if max_val > 0.0 && diff / max_val <= self.config.relative_epsilon {
                return true;
            }
        }

        // ULP-based comparison
        if self.config.use_ulp_comparison && self.config.ulp_tolerance > 0 {
            let ulp_diff = ulp_distance(a, b);
            if ulp_diff <= self.config.ulp_tolerance {
                return true;
            }
        }

        false
    }

    /// Check if two sets are equivalent (unordered)
    fn sets_equivalent(&self, a: &[NormalizedValue], b: &[NormalizedValue]) -> bool {
        if a.len() != b.len() {
            return false;
        }

        // For each element in a, find a matching element in b
        let mut used: HashSet<usize> = HashSet::new();

        for item_a in a {
            let found = b.iter().enumerate().any(|(i, item_b)| {
                if used.contains(&i) {
                    return false;
                }
                if self.check_value_equivalence(item_a, item_b, &[]) {
                    used.insert(i);
                    true
                } else {
                    false
                }
            });

            if !found {
                return false;
            }
        }

        true
    }

    /// Compare outputs line by line
    fn compare_lines(&self, expected: &str, actual: &str) -> Vec<Difference> {
        let exp_lines: Vec<&str> = expected.lines().collect();
        let act_lines: Vec<&str> = actual.lines().collect();

        let mut differences = Vec::new();

        // Handle async reordering if enabled
        if self.config.allow_async_reordering {
            let (exp_regions, act_regions) = self.extract_async_regions(&exp_lines, &act_lines);

            for (region_name, exp_region) in &exp_regions {
                if let Some(act_region) = act_regions.get(region_name) {
                    // Sort and compare within region
                    let mut exp_sorted: Vec<_> = exp_region.clone();
                    let mut act_sorted: Vec<_> = act_region.clone();
                    exp_sorted.sort();
                    act_sorted.sort();

                    if exp_sorted != act_sorted {
                        differences.push(Difference {
                            location: DiffLocation::Global,
                            kind: DiffKind::OrderMismatch,
                            expected: exp_sorted.join("\n"),
                            actual: act_sorted.join("\n"),
                            severity: DiffSeverity::Warning,
                        });
                    }
                } else {
                    differences.push(Difference {
                        location: DiffLocation::Global,
                        kind: DiffKind::Missing,
                        expected: format!("async region: {}", region_name),
                        actual: String::new(),
                        severity: DiffSeverity::Error,
                    });
                }
            }

            if differences.is_empty() {
                return differences;
            }
        }

        // Standard line comparison using LCS
        let lcs = self.compute_lcs(&exp_lines, &act_lines);
        let mut exp_idx = 0;
        let mut act_idx = 0;
        let mut lcs_idx = 0;

        while exp_idx < exp_lines.len() || act_idx < act_lines.len() {
            if lcs_idx < lcs.len()
                && exp_idx < exp_lines.len()
                && exp_lines[exp_idx] == lcs[lcs_idx]
            {
                if act_idx < act_lines.len() && act_lines[act_idx] == lcs[lcs_idx] {
                    // Lines match
                    exp_idx += 1;
                    act_idx += 1;
                    lcs_idx += 1;
                } else if act_idx < act_lines.len() {
                    // Extra line in actual
                    differences.push(Difference {
                        location: DiffLocation::Line(act_idx + 1),
                        kind: DiffKind::Extra,
                        expected: String::new(),
                        actual: act_lines[act_idx].to_string(),
                        severity: DiffSeverity::Error,
                    });
                    act_idx += 1;
                } else {
                    break;
                }
            } else if exp_idx < exp_lines.len() && act_idx < act_lines.len() {
                // Lines differ
                differences.push(Difference {
                    location: DiffLocation::Line(exp_idx + 1),
                    kind: DiffKind::LineDiff,
                    expected: exp_lines[exp_idx].to_string(),
                    actual: act_lines[act_idx].to_string(),
                    severity: self.classify_line_diff(exp_lines[exp_idx], act_lines[act_idx]),
                });
                exp_idx += 1;
                act_idx += 1;
            } else if exp_idx < exp_lines.len() {
                // Missing line in actual
                differences.push(Difference {
                    location: DiffLocation::Line(exp_idx + 1),
                    kind: DiffKind::Missing,
                    expected: exp_lines[exp_idx].to_string(),
                    actual: String::new(),
                    severity: DiffSeverity::Error,
                });
                exp_idx += 1;
            } else if act_idx < act_lines.len() {
                // Extra line in actual
                differences.push(Difference {
                    location: DiffLocation::Line(act_idx + 1),
                    kind: DiffKind::Extra,
                    expected: String::new(),
                    actual: act_lines[act_idx].to_string(),
                    severity: DiffSeverity::Error,
                });
                act_idx += 1;
            }
        }

        differences
    }

    /// Extract async regions from lines
    fn extract_async_regions<'a>(
        &self,
        exp_lines: &[&'a str],
        act_lines: &[&'a str],
    ) -> (HashMap<String, Vec<&'a str>>, HashMap<String, Vec<&'a str>>) {
        let mut exp_regions: HashMap<String, Vec<&str>> = HashMap::new();
        let mut act_regions: HashMap<String, Vec<&str>> = HashMap::new();

        fn extract<'a>(lines: &[&'a str], markers: &[String]) -> HashMap<String, Vec<&'a str>> {
            let mut regions: HashMap<String, Vec<&str>> = HashMap::new();
            let mut current_region: Option<String> = None;

            for line in lines {
                for marker in markers {
                    if line.contains(marker) {
                        if line.contains(":start") {
                            current_region = Some(
                                line.trim_start_matches(|c: char| !c.is_alphanumeric())
                                    .to_string(),
                            );
                        } else if line.contains(":end") {
                            current_region = None;
                        }
                    }
                }

                if let Some(ref region) = current_region {
                    regions.entry(region.clone()).or_default().push(*line);
                }
            }

            regions
        }

        (
            extract(exp_lines, &self.config.async_region_markers),
            extract(act_lines, &self.config.async_region_markers),
        )
    }

    /// Compute longest common subsequence
    fn compute_lcs<'a>(&self, a: &[&'a str], b: &[&'a str]) -> Vec<&'a str> {
        let m = a.len();
        let n = b.len();

        // DP table
        let mut dp = vec![vec![0; n + 1]; m + 1];

        for i in 1..=m {
            for j in 1..=n {
                if a[i - 1] == b[j - 1] {
                    dp[i][j] = dp[i - 1][j - 1] + 1;
                } else {
                    dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
                }
            }
        }

        // Backtrack to find LCS
        let mut lcs = Vec::new();
        let mut i = m;
        let mut j = n;

        while i > 0 && j > 0 {
            if a[i - 1] == b[j - 1] {
                lcs.push(a[i - 1]);
                i -= 1;
                j -= 1;
            } else if dp[i - 1][j] > dp[i][j - 1] {
                i -= 1;
            } else {
                j -= 1;
            }
        }

        lcs.reverse();
        lcs
    }

    /// Classify the severity of a line difference
    fn classify_line_diff(&self, expected: &str, actual: &str) -> DiffSeverity {
        // If only whitespace differs
        if expected.split_whitespace().collect::<Vec<_>>()
            == actual.split_whitespace().collect::<Vec<_>>()
        {
            return DiffSeverity::Info;
        }

        // If only punctuation differs
        let exp_alphanum: String = expected.chars().filter(|c| c.is_alphanumeric()).collect();
        let act_alphanum: String = actual.chars().filter(|c| c.is_alphanumeric()).collect();
        if exp_alphanum == act_alphanum {
            return DiffSeverity::Warning;
        }

        // If numbers differ slightly (might be float precision)
        let exp_nums: Vec<f64> = extract_numbers(expected);
        let act_nums: Vec<f64> = extract_numbers(actual);
        if exp_nums.len() == act_nums.len()
            && exp_nums
                .iter()
                .zip(act_nums.iter())
                .all(|(a, b)| self.floats_equivalent(*a, *b))
        {
            return DiffSeverity::Warning;
        }

        DiffSeverity::Error
    }
}

impl Default for SemanticEquivalenceChecker {
    fn default() -> Self {
        Self::default_config()
    }
}

/// Extract all numbers from a string
fn extract_numbers(s: &str) -> Vec<f64> {
    let num_pattern = regex::Regex::new(r"-?\d+\.?\d*([eE][+-]?\d+)?").unwrap();
    num_pattern
        .find_iter(s)
        .filter_map(|m| m.as_str().parse().ok())
        .collect()
}

/// Compute ULP (Units in Last Place) distance between two floats
fn ulp_distance(a: f64, b: f64) -> u64 {
    // Handle special cases
    if a.is_nan() || b.is_nan() {
        return u64::MAX;
    }
    if a.is_infinite() || b.is_infinite() {
        if a == b {
            return 0;
        }
        return u64::MAX;
    }

    // Handle sign differences
    let a_bits = a.to_bits() as i64;
    let b_bits = b.to_bits() as i64;

    // Convert to signed magnitude representation
    let a_signed = if a_bits < 0 {
        i64::MIN - a_bits
    } else {
        a_bits
    };
    let b_signed = if b_bits < 0 {
        i64::MIN - b_bits
    } else {
        b_bits
    };

    (a_signed - b_signed).unsigned_abs()
}

/// Flush denormalized numbers to zero
fn flush_denormal(x: f64) -> f64 {
    // Check if the number is denormalized (subnormal)
    // A denormal f64 has exponent bits all zero but mantissa non-zero
    if x != 0.0 && x.abs() < f64::MIN_POSITIVE {
        0.0
    } else {
        x
    }
}

/// Check if a float is denormalized
pub fn is_denormal(x: f64) -> bool {
    x != 0.0 && x.abs() < f64::MIN_POSITIVE
}

/// Compare two floats with detailed result
pub fn compare_floats(a: f64, b: f64, config: &EquivalenceConfig) -> FloatComparisonResult {
    // Handle NaN
    if a.is_nan() && b.is_nan() {
        return match config.nan_handling {
            NaNHandling::TreatAsEqual => FloatComparisonResult::Equal,
            NaNHandling::Ieee754Strict => FloatComparisonResult::NaNMismatch,
            NaNHandling::ExactBitPattern => {
                if a.to_bits() == b.to_bits() {
                    FloatComparisonResult::Equal
                } else {
                    FloatComparisonResult::NaNPayloadMismatch {
                        a_bits: a.to_bits(),
                        b_bits: b.to_bits(),
                    }
                }
            }
        };
    }

    if a.is_nan() || b.is_nan() {
        return FloatComparisonResult::NaNMismatch;
    }

    // Handle infinity
    if a.is_infinite() || b.is_infinite() {
        if a == b {
            return FloatComparisonResult::Equal;
        }
        return FloatComparisonResult::InfinityMismatch {
            a_is_inf: a.is_infinite(),
            b_is_inf: b.is_infinite(),
        };
    }

    // Handle exact equality
    if a == b {
        return FloatComparisonResult::Equal;
    }

    let diff = (a - b).abs();
    let ulp_diff = ulp_distance(a, b);

    // Check if within tolerance
    let within_abs_epsilon = diff <= config.float_epsilon;
    let within_rel_epsilon = if config.use_relative_epsilon {
        let max_val = a.abs().max(b.abs());
        max_val > 0.0 && diff / max_val <= config.relative_epsilon
    } else {
        false
    };
    let within_ulp = config.use_ulp_comparison && ulp_diff <= config.ulp_tolerance;

    if within_abs_epsilon || within_rel_epsilon || within_ulp {
        FloatComparisonResult::WithinTolerance {
            absolute_diff: diff,
            relative_diff: if a.abs().max(b.abs()) > 0.0 {
                diff / a.abs().max(b.abs())
            } else {
                0.0
            },
            ulp_diff,
        }
    } else {
        FloatComparisonResult::OutOfTolerance {
            absolute_diff: diff,
            relative_diff: if a.abs().max(b.abs()) > 0.0 {
                diff / a.abs().max(b.abs())
            } else {
                0.0
            },
            ulp_diff,
        }
    }
}

/// Result of comparing two floats
#[derive(Debug, Clone, PartialEq)]
pub enum FloatComparisonResult {
    /// Floats are exactly equal
    Equal,
    /// Floats differ but within tolerance
    WithinTolerance {
        absolute_diff: f64,
        relative_diff: f64,
        ulp_diff: u64,
    },
    /// Floats differ beyond tolerance
    OutOfTolerance {
        absolute_diff: f64,
        relative_diff: f64,
        ulp_diff: u64,
    },
    /// One or both values are NaN (unequal per IEEE 754)
    NaNMismatch,
    /// NaN values have different payloads
    NaNPayloadMismatch { a_bits: u64, b_bits: u64 },
    /// Infinity mismatch
    InfinityMismatch { a_is_inf: bool, b_is_inf: bool },
}

impl FloatComparisonResult {
    /// Check if the comparison is considered equal
    pub fn is_equal(&self) -> bool {
        matches!(
            self,
            FloatComparisonResult::Equal | FloatComparisonResult::WithinTolerance { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let checker = SemanticEquivalenceChecker::default_config();
        let result = checker.check("hello world", "hello world");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_float_tolerance() {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
            float_epsilon: 1e-6,
            ..Default::default()
        });

        let result = checker.check("value: 3.141592653589", "value: 3.141592653590");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_float_mismatch() {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
            float_epsilon: 1e-15,
            use_relative_epsilon: false,
            ..Default::default()
        });

        let result = checker.check("value: 1.0", "value: 1.1");
        assert!(!result.is_equivalent());
    }

    #[test]
    fn test_ignore_whitespace() {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
            ignore_whitespace: true,
            ..Default::default()
        });

        let result = checker.check("hello   world", "hello world");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_ignore_empty_lines() {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
            ignore_empty_lines: true,
            ..Default::default()
        });

        let result = checker.check("line1\n\nline2", "line1\nline2");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_set_equivalence() {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig {
            allow_unordered_collections: true,
            ..Default::default()
        });

        let result = checker.check("{1, 2, 3}", "{3, 1, 2}");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_list_order_matters() {
        let checker = SemanticEquivalenceChecker::default_config();

        let result = checker.check("[1, 2, 3]", "[3, 1, 2]");
        assert!(!result.is_equivalent());
    }

    #[test]
    fn test_nan_equivalence() {
        let checker = SemanticEquivalenceChecker::default_config();

        let result = checker.check("NaN", "NaN");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_difference_location() {
        let checker = SemanticEquivalenceChecker::default_config();

        let result = checker.check("line1\nline2\nline3", "line1\nmodified\nline3");

        match result {
            EquivalenceResult::Different(diffs) => {
                assert!(!diffs.is_empty());
                assert!(matches!(diffs[0].location, DiffLocation::Line(2)));
            }
            _ => panic!("Expected difference"),
        }
    }
}
