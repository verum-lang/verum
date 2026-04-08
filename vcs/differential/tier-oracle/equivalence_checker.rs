//! Result Equivalence Checker for Tier Oracle
//!
//! This module provides comprehensive semantic equivalence checking between
//! outputs from different execution tiers, handling platform-specific variations
//! and acceptable differences.

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Configuration for equivalence checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivalenceCheckConfig {
    /// Float comparison epsilon for absolute difference
    pub float_abs_epsilon: f64,

    /// Float comparison epsilon for relative difference
    pub float_rel_epsilon: f64,

    /// ULP (Units in Last Place) tolerance for float comparison
    pub float_ulp_tolerance: u64,

    /// Whether to allow unordered collection comparison
    pub allow_unordered_collections: bool,

    /// Whether to allow async output reordering within marked sections
    pub allow_async_reordering: bool,

    /// Maximum reorder distance for async outputs
    pub max_reorder_distance: usize,

    /// Patterns that mark async/unordered sections
    pub unordered_markers: Vec<String>,

    /// Whether to treat NaN values as equal
    pub nan_equals_nan: bool,

    /// Whether to treat positive and negative zero as equal
    pub zero_sign_sensitive: bool,

    /// Custom equivalence rules
    pub custom_rules: Vec<EquivalenceRule>,
}

impl Default for EquivalenceCheckConfig {
    fn default() -> Self {
        Self {
            float_abs_epsilon: 1e-10,
            float_rel_epsilon: 1e-10,
            float_ulp_tolerance: 4,
            allow_unordered_collections: true,
            allow_async_reordering: false,
            max_reorder_distance: 10,
            unordered_markers: vec![
                "@unordered:".to_string(),
                "@async:".to_string(),
            ],
            nan_equals_nan: true,
            zero_sign_sensitive: false,
            custom_rules: vec![],
        }
    }
}

/// A custom equivalence rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivalenceRule {
    /// Name of the rule
    pub name: String,

    /// Pattern to match in expected output
    pub expected_pattern: String,

    /// Pattern to match in actual output
    pub actual_pattern: String,

    /// Whether to treat matches as equivalent
    pub treat_as_equivalent: bool,
}

/// Result of equivalence check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EquivalenceResult {
    /// Outputs are equivalent
    Equivalent,

    /// Outputs are equivalent after semantic normalization
    SemanticEquivalent {
        normalizations: Vec<String>,
    },

    /// Outputs differ
    Different {
        differences: Vec<EquivalenceDiff>,
    },
}

impl EquivalenceResult {
    /// Whether the result indicates equivalence
    pub fn is_equivalent(&self) -> bool {
        matches!(self, EquivalenceResult::Equivalent | EquivalenceResult::SemanticEquivalent { .. })
    }
}

/// A difference found during equivalence checking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquivalenceDiff {
    /// Location of the difference
    pub location: DiffLocation,

    /// Expected value
    pub expected: String,

    /// Actual value
    pub actual: String,

    /// Type of difference
    pub diff_type: DiffType,

    /// Severity of the difference
    pub severity: DiffSeverity,
}

/// Location of a difference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLocation {
    /// Line number (1-indexed)
    pub line: usize,

    /// Column number (1-indexed, if known)
    pub column: Option<usize>,

    /// Path within structured output (e.g., "results[0].value")
    pub path: Option<String>,
}

impl std::fmt::Display for DiffLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{}", path)
        } else if let Some(col) = self.column {
            write!(f, "line {}:{}", self.line, col)
        } else {
            write!(f, "line {}", self.line)
        }
    }
}

/// Type of difference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffType {
    /// Text content differs
    TextContent,

    /// Numeric value differs
    NumericValue,

    /// Float precision differs (may be acceptable)
    FloatPrecision,

    /// Collection element differs
    CollectionElement,

    /// Collection size differs
    CollectionSize,

    /// Collection ordering differs
    CollectionOrder,

    /// Type differs
    TypeMismatch,

    /// Value is missing
    Missing,

    /// Extra value present
    Extra,

    /// Whitespace differs
    Whitespace,

    /// NaN handling differs
    NaNHandling,

    /// Infinity handling differs
    InfinityHandling,
}

/// Severity of a difference
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DiffSeverity {
    /// Informational only, not a real difference
    Info,

    /// Minor difference that may be acceptable
    Warning,

    /// Significant difference that should be investigated
    Error,

    /// Critical difference indicating a bug
    Critical,
}

/// The equivalence checker
pub struct EquivalenceChecker {
    config: EquivalenceCheckConfig,
}

impl EquivalenceChecker {
    /// Create a new equivalence checker
    pub fn new(config: EquivalenceCheckConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(EquivalenceCheckConfig::default())
    }

    /// Check equivalence between expected and actual outputs
    pub fn check(&self, expected: &str, actual: &str) -> EquivalenceResult {
        // Quick exact match check
        if expected == actual {
            return EquivalenceResult::Equivalent;
        }

        // Try semantic comparison
        let mut normalizations = Vec::new();
        let mut differences = Vec::new();

        // Parse and compare line by line
        let expected_lines: Vec<&str> = expected.lines().collect();
        let actual_lines: Vec<&str> = actual.lines().collect();

        // Check line count
        if expected_lines.len() != actual_lines.len() {
            differences.push(EquivalenceDiff {
                location: DiffLocation {
                    line: 0,
                    column: None,
                    path: Some("line_count".to_string()),
                },
                expected: format!("{}", expected_lines.len()),
                actual: format!("{}", actual_lines.len()),
                diff_type: DiffType::CollectionSize,
                severity: DiffSeverity::Error,
            });
        }

        // Compare each line
        let max_lines = expected_lines.len().max(actual_lines.len());
        for i in 0..max_lines {
            let exp_line = expected_lines.get(i);
            let act_line = actual_lines.get(i);

            match (exp_line, act_line) {
                (Some(e), Some(a)) => {
                    if e != a {
                        if let Some(diff) = self.compare_lines(e, a, i + 1) {
                            differences.push(diff);
                        } else {
                            normalizations.push(format!("Line {}: normalized", i + 1));
                        }
                    }
                }
                (Some(e), None) => {
                    differences.push(EquivalenceDiff {
                        location: DiffLocation {
                            line: i + 1,
                            column: None,
                            path: None,
                        },
                        expected: e.to_string(),
                        actual: "<missing>".to_string(),
                        diff_type: DiffType::Missing,
                        severity: DiffSeverity::Error,
                    });
                }
                (None, Some(a)) => {
                    differences.push(EquivalenceDiff {
                        location: DiffLocation {
                            line: i + 1,
                            column: None,
                            path: None,
                        },
                        expected: "<missing>".to_string(),
                        actual: a.to_string(),
                        diff_type: DiffType::Extra,
                        severity: DiffSeverity::Error,
                    });
                }
                (None, None) => {}
            }
        }

        // Return result
        if differences.is_empty() {
            if normalizations.is_empty() {
                EquivalenceResult::Equivalent
            } else {
                EquivalenceResult::SemanticEquivalent { normalizations }
            }
        } else {
            EquivalenceResult::Different { differences }
        }
    }

    /// Compare two lines and return a diff if they don't match
    fn compare_lines(&self, expected: &str, actual: &str, line_num: usize) -> Option<EquivalenceDiff> {
        // Try float comparison
        if let Some(equiv) = self.try_float_equivalent(expected, actual) {
            if equiv {
                return None; // Semantically equivalent
            }
        }

        // Try collection comparison for array-like outputs
        if expected.starts_with('[') && actual.starts_with('[') {
            if self.are_collections_equivalent(expected, actual) {
                return None;
            }
        }

        // Try map comparison for object-like outputs
        if expected.starts_with('{') && actual.starts_with('{') {
            if self.are_maps_equivalent(expected, actual) {
                return None;
            }
        }

        // Check for whitespace-only differences
        if expected.split_whitespace().collect::<Vec<_>>() ==
           actual.split_whitespace().collect::<Vec<_>>() {
            return Some(EquivalenceDiff {
                location: DiffLocation {
                    line: line_num,
                    column: None,
                    path: None,
                },
                expected: expected.to_string(),
                actual: actual.to_string(),
                diff_type: DiffType::Whitespace,
                severity: DiffSeverity::Warning,
            });
        }

        // Apply custom rules
        for rule in &self.config.custom_rules {
            if let (Ok(exp_re), Ok(act_re)) = (
                Regex::new(&rule.expected_pattern),
                Regex::new(&rule.actual_pattern),
            ) {
                if exp_re.is_match(expected) && act_re.is_match(actual) {
                    if rule.treat_as_equivalent {
                        return None;
                    }
                }
            }
        }

        // Default: lines differ
        Some(EquivalenceDiff {
            location: DiffLocation {
                line: line_num,
                column: self.find_first_diff_column(expected, actual),
                path: None,
            },
            expected: expected.to_string(),
            actual: actual.to_string(),
            diff_type: DiffType::TextContent,
            severity: DiffSeverity::Error,
        })
    }

    /// Try to compare as floats, returns Some(true) if equivalent, Some(false) if different, None if not floats
    fn try_float_equivalent(&self, expected: &str, actual: &str) -> Option<bool> {
        let exp_trimmed = expected.trim();
        let act_trimmed = actual.trim();

        // Parse as floats
        let exp_float: Result<f64, _> = exp_trimmed.parse();
        let act_float: Result<f64, _> = act_trimmed.parse();

        match (exp_float, act_float) {
            (Ok(e), Ok(a)) => Some(self.floats_equivalent(e, a)),
            _ => {
                // Try parsing space-separated values
                let exp_parts: Vec<&str> = exp_trimmed.split_whitespace().collect();
                let act_parts: Vec<&str> = act_trimmed.split_whitespace().collect();

                if exp_parts.len() != act_parts.len() {
                    return None;
                }

                // Check if all parts are equivalent floats
                let mut all_floats = true;
                let mut all_equiv = true;

                for (e, a) in exp_parts.iter().zip(act_parts.iter()) {
                    match (e.parse::<f64>(), a.parse::<f64>()) {
                        (Ok(ef), Ok(af)) => {
                            if !self.floats_equivalent(ef, af) {
                                all_equiv = false;
                            }
                        }
                        _ => {
                            all_floats = false;
                            if e != a {
                                all_equiv = false;
                            }
                        }
                    }
                }

                if all_floats || all_equiv {
                    Some(all_equiv)
                } else {
                    None
                }
            }
        }
    }

    /// Check if two floats are equivalent according to configuration
    fn floats_equivalent(&self, a: f64, b: f64) -> bool {
        // Handle NaN
        if a.is_nan() && b.is_nan() {
            return self.config.nan_equals_nan;
        }

        // Handle infinities
        if a.is_infinite() && b.is_infinite() {
            return a.is_sign_positive() == b.is_sign_positive();
        }

        // Handle zeros
        if a == 0.0 && b == 0.0 {
            if self.config.zero_sign_sensitive {
                return a.is_sign_positive() == b.is_sign_positive();
            }
            return true;
        }

        // Absolute difference check
        let abs_diff = (a - b).abs();
        if abs_diff <= self.config.float_abs_epsilon {
            return true;
        }

        // Relative difference check
        let max_abs = a.abs().max(b.abs());
        if max_abs > 0.0 && abs_diff / max_abs <= self.config.float_rel_epsilon {
            return true;
        }

        // ULP comparison
        if self.config.float_ulp_tolerance > 0 {
            let ulp_diff = self.ulp_difference(a, b);
            if ulp_diff <= self.config.float_ulp_tolerance {
                return true;
            }
        }

        false
    }

    /// Calculate ULP (Units in Last Place) difference between two floats
    fn ulp_difference(&self, a: f64, b: f64) -> u64 {
        let a_bits = a.to_bits() as i64;
        let b_bits = b.to_bits() as i64;
        (a_bits - b_bits).unsigned_abs()
    }

    /// Check if two array-like outputs are equivalent (possibly unordered)
    fn are_collections_equivalent(&self, expected: &str, actual: &str) -> bool {
        // Simple bracket-matching parser
        let exp_elements = self.parse_collection_elements(expected);
        let act_elements = self.parse_collection_elements(actual);

        if exp_elements.len() != act_elements.len() {
            return false;
        }

        if self.config.allow_unordered_collections {
            // Compare as sets
            let exp_set: HashSet<_> = exp_elements.iter().map(|s| s.trim()).collect();
            let act_set: HashSet<_> = act_elements.iter().map(|s| s.trim()).collect();
            exp_set == act_set
        } else {
            // Compare in order
            exp_elements.iter().zip(act_elements.iter())
                .all(|(e, a)| e.trim() == a.trim())
        }
    }

    /// Check if two map-like outputs are equivalent
    fn are_maps_equivalent(&self, expected: &str, actual: &str) -> bool {
        let exp_entries = self.parse_map_entries(expected);
        let act_entries = self.parse_map_entries(actual);

        if exp_entries.len() != act_entries.len() {
            return false;
        }

        // Maps are inherently unordered
        let exp_map: HashMap<_, _> = exp_entries.into_iter().collect();
        let act_map: HashMap<_, _> = act_entries.into_iter().collect();

        exp_map == act_map
    }

    /// Parse collection elements from a string like "[1, 2, 3]"
    fn parse_collection_elements(&self, s: &str) -> Vec<String> {
        let trimmed = s.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return vec![];
        }

        let inner = &trimmed[1..trimmed.len()-1];
        let mut elements = Vec::new();
        let mut current = String::new();
        let mut depth = 0;

        for c in inner.chars() {
            match c {
                '[' | '{' | '(' => {
                    depth += 1;
                    current.push(c);
                }
                ']' | '}' | ')' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    if !current.trim().is_empty() {
                        elements.push(current.trim().to_string());
                    }
                    current.clear();
                }
                _ => current.push(c),
            }
        }

        if !current.trim().is_empty() {
            elements.push(current.trim().to_string());
        }

        elements
    }

    /// Parse map entries from a string like "{a: 1, b: 2}"
    fn parse_map_entries(&self, s: &str) -> Vec<(String, String)> {
        let trimmed = s.trim();
        if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
            return vec![];
        }

        let inner = &trimmed[1..trimmed.len()-1];
        let mut entries = Vec::new();
        let mut current = String::new();
        let mut depth = 0;

        for c in inner.chars() {
            match c {
                '[' | '{' | '(' => {
                    depth += 1;
                    current.push(c);
                }
                ']' | '}' | ')' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    if let Some((k, v)) = self.parse_map_entry(&current) {
                        entries.push((k, v));
                    }
                    current.clear();
                }
                _ => current.push(c),
            }
        }

        if let Some((k, v)) = self.parse_map_entry(&current) {
            entries.push((k, v));
        }

        entries
    }

    /// Parse a single map entry like "key: value"
    fn parse_map_entry(&self, s: &str) -> Option<(String, String)> {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() == 2 {
            Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
        } else {
            None
        }
    }

    /// Find the first column where two strings differ
    fn find_first_diff_column(&self, a: &str, b: &str) -> Option<usize> {
        for (i, (ca, cb)) in a.chars().zip(b.chars()).enumerate() {
            if ca != cb {
                return Some(i + 1);
            }
        }
        if a.len() != b.len() {
            return Some(a.len().min(b.len()) + 1);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let checker = EquivalenceChecker::with_defaults();
        let result = checker.check("hello\nworld", "hello\nworld");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_float_precision() {
        let checker = EquivalenceChecker::with_defaults();
        let result = checker.check("3.3333333333333335", "3.333333333333333");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_different_values() {
        let checker = EquivalenceChecker::with_defaults();
        let result = checker.check("42", "43");
        assert!(!result.is_equivalent());
    }

    #[test]
    fn test_unordered_collection() {
        let config = EquivalenceCheckConfig {
            allow_unordered_collections: true,
            ..Default::default()
        };
        let checker = EquivalenceChecker::new(config);
        let result = checker.check("[1, 2, 3]", "[3, 1, 2]");
        assert!(result.is_equivalent());
    }

    #[test]
    fn test_nan_handling() {
        let config = EquivalenceCheckConfig {
            nan_equals_nan: true,
            ..Default::default()
        };
        let checker = EquivalenceChecker::new(config);
        assert!(checker.floats_equivalent(f64::NAN, f64::NAN));
    }

    #[test]
    fn test_ulp_comparison() {
        let checker = EquivalenceChecker::with_defaults();
        // These differ by 1 ULP
        let a = 1.0;
        let b = f64::from_bits(a.to_bits() + 1);
        assert!(checker.floats_equivalent(a, b));
    }
}
