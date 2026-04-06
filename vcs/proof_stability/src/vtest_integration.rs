//! Integration with VCS vtest runner.
//!
//! This module provides integration between the proof stability infrastructure
//! and the vtest test runner, enabling stability testing for @test: verify-pass tests.

use crate::{
    ProofCategory, ProofId, StabilityStatus,
    metrics::ProofMetrics,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use verum_common::{List, Text};

/// Integration configuration for vtest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VTestIntegrationConfig {
    /// Enable stability testing for verify-pass tests
    pub enabled: bool,
    /// Number of stability runs per proof
    pub stability_runs: usize,
    /// Seeds to use for stability testing
    pub seeds: List<u64>,
    /// Fail on flaky proofs
    pub fail_on_flaky: bool,
    /// Stability threshold (0-100)
    pub stability_threshold: f64,
    /// Report flaky proofs
    pub report_flaky: bool,
    /// Cache stability results
    pub cache_results: bool,
}

impl Default for VTestIntegrationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stability_runs: 3,
            seeds: vec![42, 123, 456].into(),
            fail_on_flaky: false,
            stability_threshold: 95.0,
            report_flaky: true,
            cache_results: true,
        }
    }
}

/// Directive for proof stability testing.
/// Can be embedded in .vr files as comments.
#[derive(Debug, Clone)]
pub struct StabilityDirective {
    /// Expected stability status
    pub expected_status: Option<StabilityStatus>,
    /// Minimum stability percentage
    pub min_stability: Option<f64>,
    /// Proof category hint
    pub category: Option<ProofCategory>,
    /// Skip stability testing for this proof
    pub skip: bool,
    /// Custom seeds for this proof
    pub seeds: Option<List<u64>>,
    /// Custom timeout for this proof
    pub timeout_ms: Option<u64>,
}

impl StabilityDirective {
    /// Parse stability directive from test file content.
    pub fn parse(content: &str) -> Self {
        let mut directive = Self {
            expected_status: None,
            min_stability: None,
            category: None,
            skip: false,
            seeds: None,
            timeout_ms: None,
        };

        for line in content.lines() {
            let line = line.trim();
            if !line.starts_with("//") {
                continue;
            }

            let comment = line.trim_start_matches("//").trim();

            // Parse @stability-skip
            if comment.contains("@stability-skip") {
                directive.skip = true;
            }

            // Parse @stability-expect: stable|flaky|unknown
            if let Some(rest) = comment.strip_prefix("@stability-expect:") {
                let status = rest.trim().to_lowercase();
                directive.expected_status = match status.as_str() {
                    "stable" => Some(StabilityStatus::Stable),
                    "flaky" => Some(StabilityStatus::Flaky),
                    "unknown" => Some(StabilityStatus::Unknown),
                    "timeout-unstable" => Some(StabilityStatus::TimeoutUnstable),
                    _ => None,
                };
            }

            // Parse @stability-min: 95
            if let Some(rest) = comment.strip_prefix("@stability-min:") {
                directive.min_stability = rest.trim().parse().ok();
            }

            // Parse @stability-category: arithmetic|quantifier|...
            if let Some(rest) = comment.strip_prefix("@stability-category:") {
                directive.category = ProofCategory::from_str(rest.trim());
            }

            // Parse @stability-seeds: 42,123,456
            if let Some(rest) = comment.strip_prefix("@stability-seeds:") {
                directive.seeds = Some(
                    rest.trim()
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect(),
                );
            }

            // Parse @stability-timeout: 60000
            if let Some(rest) = comment.strip_prefix("@stability-timeout:") {
                directive.timeout_ms = rest.trim().parse().ok();
            }
        }

        directive
    }
}

/// Result of stability testing for a single proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityTestResult {
    /// Test file path
    pub path: Text,
    /// Proof identifier
    pub proof_id: ProofId,
    /// Stability metrics
    pub metrics: ProofMetrics,
    /// Whether stability test passed
    pub passed: bool,
    /// Failure reason (if any)
    pub failure_reason: Option<Text>,
    /// Duration of stability testing
    pub duration: Duration,
}

impl StabilityTestResult {
    /// Check if this test meets the expected directive.
    pub fn meets_directive(&self, directive: &StabilityDirective) -> bool {
        // Check expected status
        if let Some(expected) = directive.expected_status {
            if self.metrics.stability_status != expected {
                return false;
            }
        }

        // Check minimum stability
        if let Some(min) = directive.min_stability {
            if self.metrics.stability_percentage < min {
                return false;
            }
        }

        true
    }
}

/// Summary of vtest stability integration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VTestStabilitySummary {
    /// Total tests processed
    pub total_tests: usize,
    /// Tests with stability testing enabled
    pub stability_tested: usize,
    /// Tests skipped
    pub skipped: usize,
    /// Stable tests
    pub stable: usize,
    /// Flaky tests
    pub flaky: usize,
    /// Tests that failed stability requirements
    pub failed_requirements: usize,
    /// Overall stability percentage
    pub overall_stability: f64,
    /// Individual results
    pub results: List<StabilityTestResult>,
}

impl Default for VTestStabilitySummary {
    fn default() -> Self {
        Self {
            total_tests: 0,
            stability_tested: 0,
            skipped: 0,
            stable: 0,
            flaky: 0,
            failed_requirements: 0,
            overall_stability: 0.0,
            results: List::new(),
        }
    }
}

impl VTestStabilitySummary {
    /// Create a new summary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a result.
    pub fn add(&mut self, result: StabilityTestResult) {
        self.total_tests += 1;

        if result.passed {
            if result.metrics.is_stable() {
                self.stable += 1;
            } else if result.metrics.is_flaky() {
                self.flaky += 1;
            }
        } else {
            self.failed_requirements += 1;
        }

        self.stability_tested += 1;
        self.results.push(result);
    }

    /// Add a skipped test.
    pub fn add_skipped(&mut self) {
        self.total_tests += 1;
        self.skipped += 1;
    }

    /// Finalize the summary.
    pub fn finalize(&mut self) {
        if self.stability_tested > 0 {
            self.overall_stability = (self.stable as f64 / self.stability_tested as f64) * 100.0;
        } else {
            self.overall_stability = 100.0;
        }
    }

    /// Check if all stability requirements are met.
    pub fn all_passed(&self) -> bool {
        self.failed_requirements == 0
    }
}

/// Check if a test file should have stability testing.
pub fn should_test_stability(content: &str) -> bool {
    // Only test @test: verify-pass tests
    content.contains("@test: verify-pass") || content.contains("@test:verify-pass")
}

/// Extract proof category from test tags.
pub fn extract_category_from_tags(content: &str) -> ProofCategory {
    let content_lower = content.to_lowercase();

    if content_lower.contains("arithmetic") || content_lower.contains("arith") {
        ProofCategory::Arithmetic
    } else if content_lower.contains("quantifier") || content_lower.contains("forall") {
        ProofCategory::Quantifier
    } else if content_lower.contains("array") || content_lower.contains("memory") {
        ProofCategory::Array
    } else if content_lower.contains("recursive") || content_lower.contains("termination") {
        ProofCategory::Recursive
    } else if content_lower.contains("bitvector") || content_lower.contains("bv") {
        ProofCategory::BitVector
    } else if content_lower.contains("string") {
        ProofCategory::String
    } else {
        ProofCategory::Mixed
    }
}

/// Format stability result for vtest output.
pub fn format_stability_result(result: &StabilityTestResult, use_colors: bool) -> Text {
    let status = match result.metrics.stability_status {
        StabilityStatus::Stable => {
            if use_colors {
                "\x1b[32mstable\x1b[0m".to_string()
            } else {
                "stable".to_string()
            }
        }
        StabilityStatus::Flaky => {
            if use_colors {
                "\x1b[33mflaky\x1b[0m".to_string()
            } else {
                "flaky".to_string()
            }
        }
        StabilityStatus::Unknown => "unknown".to_string(),
        StabilityStatus::TimeoutUnstable => {
            if use_colors {
                "\x1b[31mtimeout-unstable\x1b[0m".to_string()
            } else {
                "timeout-unstable".to_string()
            }
        }
    };

    format!(
        "[{}] {:.1}% ({}/{} verified)",
        status,
        result.metrics.stability_percentage,
        result.metrics.verified_count,
        result.metrics.attempt_count
    ).into()
}

/// Example vtest integration usage.
///
/// ```rust,ignore
/// use proof_stability::vtest_integration::*;
///
/// async fn run_stability_test(
///     test_path: &Path,
///     config: &StabilityConfig,
/// ) -> Result<StabilityTestResult, StabilityError> {
///     let content = std::fs::read_to_string(test_path)?;
///
///     // Check if stability testing applies
///     if !should_test_stability(&content) {
///         return Ok(/* skipped result */);
///     }
///
///     // Parse directive
///     let directive = StabilityDirective::parse(&content);
///     if directive.skip {
///         return Ok(/* skipped result */);
///     }
///
///     // Run stability test...
///     // ...
/// }
/// ```

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_stability_directive() {
        let content = r#"
// @test: verify-pass
// @stability-expect: stable
// @stability-min: 95
// @stability-category: arithmetic
fn test() {}
"#;

        let directive = StabilityDirective::parse(content);
        assert_eq!(directive.expected_status, Some(StabilityStatus::Stable));
        assert_eq!(directive.min_stability, Some(95.0));
        assert_eq!(directive.category, Some(ProofCategory::Arithmetic));
        assert!(!directive.skip);
    }

    #[test]
    fn test_parse_stability_skip() {
        let content = r#"
// @test: verify-pass
// @stability-skip
fn test() {}
"#;

        let directive = StabilityDirective::parse(content);
        assert!(directive.skip);
    }

    #[test]
    fn test_should_test_stability() {
        assert!(should_test_stability("// @test: verify-pass"));
        assert!(should_test_stability("// @test:verify-pass"));
        assert!(!should_test_stability("// @test: run"));
        assert!(!should_test_stability("// @test: parse-pass"));
    }

    #[test]
    fn test_extract_category() {
        assert_eq!(
            extract_category_from_tags("@tags: arithmetic, simple"),
            ProofCategory::Arithmetic
        );
        assert_eq!(
            extract_category_from_tags("@tags: quantifier, forall"),
            ProofCategory::Quantifier
        );
        assert_eq!(
            extract_category_from_tags("@tags: memory, array"),
            ProofCategory::Array
        );
        assert_eq!(
            extract_category_from_tags("@tags: other"),
            ProofCategory::Mixed
        );
    }
}
