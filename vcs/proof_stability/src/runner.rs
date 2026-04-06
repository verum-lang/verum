//! Proof stability runner.
//!
//! Orchestrates proof discovery, execution, and stability analysis.

use crate::{
    ProofAttempt, ProofCategory, ProofId, StabilityError,
    config::StabilityConfig,
    metrics::{ProofMetrics, StabilityMetrics},
    recorder::{ProofRecorder, ProofRecording},
    regression::RegressionDetector,
    report::StabilityReport,
    solver::DeterministicSolver,
};
use chrono::Utc;
use glob::glob;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use verum_common::{List, Text};

/// Summary of a stability test run.
#[derive(Debug, Clone, Default)]
pub struct StabilityRunSummary {
    /// Total proofs tested
    pub total_proofs: usize,
    /// Proofs that are stable
    pub stable_proofs: usize,
    /// Proofs that are flaky
    pub flaky_proofs: usize,
    /// Proofs with unknown stability
    pub unknown_proofs: usize,
    /// Total proof attempts
    pub total_attempts: usize,
    /// Overall stability percentage
    pub stability_percentage: f64,
    /// Total execution time
    pub execution_time: Duration,
    /// Errors encountered
    pub errors: List<Text>,
}

/// Proof stability runner.
pub struct ProofStabilityRunner {
    config: StabilityConfig,
    recorder: ProofRecorder,
    regression_detector: RegressionDetector,
}

impl ProofStabilityRunner {
    /// Create a new runner with the given configuration.
    pub fn new(config: StabilityConfig) -> Self {
        let recorder = ProofRecorder::new(config.cache.clone(), config.solver.clone());
        let regression_detector = RegressionDetector::new(config.thresholds.clone());

        Self {
            config,
            recorder,
            regression_detector,
        }
    }

    /// Initialize the runner.
    pub async fn initialize(&mut self) -> Result<(), StabilityError> {
        self.recorder.initialize().await?;

        // Load baseline if configured
        if let Some(ref path) = self.config.reporting.baseline_path {
            self.regression_detector.load_baseline(path)?;
        }

        Ok(())
    }

    /// Discover proof test files.
    pub fn discover_tests(&self) -> Result<List<PathBuf>, StabilityError> {
        let mut tests = List::new();

        for base_path in &self.config.execution.test_paths {
            let pattern = base_path
                .join(&self.config.execution.test_pattern)
                .to_string_lossy()
                .to_string();

            for entry in glob(&pattern)
                .map_err(|e| StabilityError::ConfigError(format!("Invalid glob pattern: {}", e).into()))?
            {
                let path = entry.map_err(|e| StabilityError::IoError(e.into_error()))?;

                // Check exclusions
                let path_str = path.to_string_lossy();
                let excluded = self.config.execution.exclude_patterns.iter().any(|p| {
                    path_str.contains(p.as_str())
                        || glob::Pattern::new(p)
                            .map(|pat| pat.matches(&path_str))
                            .unwrap_or(false)
                });

                if !excluded && path.is_file() {
                    tests.push(path);
                }
            }
        }

        Ok(tests)
    }

    /// Parse a test file and extract proof information.
    pub fn parse_test_file(&self, path: &Path) -> Result<Option<ProofTestInfo>, StabilityError> {
        let content = std::fs::read_to_string(path)?;

        // Check for @test: verify-pass directive
        if !content.contains("@test: verify-pass") && !content.contains("@test:verify-pass") {
            return Ok(None);
        }

        let proof_id = ProofId::new(
            path.to_string_lossy().to_string().into(),
            extract_scope(&content).unwrap_or_else(|| "main".to_string().into()),
            find_first_assert_line(&content).unwrap_or(1),
            extract_description(&content).unwrap_or_else(|| "proof".to_string().into()),
        );

        let category = extract_category(&content).unwrap_or(ProofCategory::Mixed);
        let formula = extract_smt_formula(&content);

        Ok(Some(ProofTestInfo {
            path: path.to_path_buf(),
            proof_id,
            category,
            formula,
            content: content.into(),
        }))
    }

    /// Run stability tests on all discovered files.
    pub async fn run(&mut self) -> Result<(StabilityMetrics, StabilityRunSummary), StabilityError> {
        let start = Instant::now();
        let tests = self.discover_tests()?;

        // Parse all test files
        let mut proof_tests: List<ProofTestInfo> = List::new();
        for path in &tests {
            if let Some(info) = self.parse_test_file(path)? {
                proof_tests.push(info);
            }
        }

        if proof_tests.is_empty() {
            return Ok((StabilityMetrics::new(), StabilityRunSummary::default()));
        }

        // Run stability tests
        let seeds = self.config.solver.stability_seeds();
        let recordings = self.run_stability_tests(&proof_tests, &seeds).await?;

        // Compute metrics
        let mut all_metrics: List<ProofMetrics> = List::new();
        for recording in &recordings {
            let attempts: List<ProofAttempt> = recording
                .attempts
                .iter()
                .map(|r| ProofAttempt {
                    proof_id: recording.proof_id.clone(),
                    category: recording.category,
                    seed: r.seed,
                    solver: r.solver.clone(),
                    solver_version: r.solver_version.clone(),
                    outcome: r.outcome.clone(),
                    duration: r.duration,
                    timestamp: Utc::now(),
                    metadata: HashMap::new(),
                })
                .collect();

            let metrics = ProofMetrics::from_attempts(
                recording.proof_id.clone(),
                recording.category,
                &attempts,
                &self.config.thresholds,
            );
            all_metrics.push(metrics);
        }

        // Aggregate metrics
        let mut aggregated = StabilityMetrics::new();
        for m in &all_metrics {
            aggregated.add_proof(m);
        }
        aggregated.finalize();

        // Build summary
        let summary = StabilityRunSummary {
            total_proofs: aggregated.total_proofs,
            stable_proofs: aggregated.stable_count,
            flaky_proofs: aggregated.flaky_count,
            unknown_proofs: aggregated.unknown_count,
            total_attempts: aggregated.total_attempts,
            stability_percentage: aggregated.overall_stability,
            execution_time: start.elapsed(),
            errors: List::new(),
        };

        Ok((aggregated, summary))
    }

    /// Run stability tests on a list of proof tests.
    async fn run_stability_tests(
        &mut self,
        tests: &[ProofTestInfo],
        seeds: &[u64],
    ) -> Result<List<ProofRecording>, StabilityError> {
        let mut recordings = List::new();

        for test in tests {
            // For now, we use a mock SMT formula since real verification
            // would require the full Verum compiler pipeline
            let formula = test
                .formula
                .as_ref()
                .cloned()
                .unwrap_or_else(|| generate_mock_formula(&test.content, &test.category));

            let recording_key =
                self.recorder
                    .start_recording(test.proof_id.clone(), test.category, formula);

            // Run multiple attempts with different seeds
            for &seed in seeds {
                match self.recorder.record_attempt(&recording_key, seed).await {
                    Ok(_) => {}
                    Err(e) => {
                        // Log error but continue with other attempts
                        eprintln!("Warning: Proof attempt failed: {}", e);
                    }
                }
            }

            if let Ok(recording) = self.recorder.finish_recording(&recording_key) {
                recordings.push(recording);
            }
        }

        Ok(recordings)
    }

    /// Run and generate a full report.
    pub async fn run_and_report(&mut self) -> Result<StabilityReport, StabilityError> {
        let (metrics, summary) = self.run().await?;

        // Get solver version
        let solver = DeterministicSolver::new(self.config.solver.clone());
        let solver_version = solver
            .get_version(&self.config.solver.default_solver)
            .await
            .ok();

        let mut report = StabilityReport::new(metrics)
            .with_title("VCS Proof Stability Report".to_string().into())
            .with_solver(self.config.solver.default_solver.clone(), solver_version)
            .with_execution_time(summary.execution_time);

        // Add regressions if baseline is configured
        if self.config.reporting.generate_regression_report {
            // Would need current metrics for regression detection
            // For now, skip regression analysis
        }

        report.compute_exit_code(self.config.thresholds.stable_threshold * 100.0, false);

        Ok(report)
    }

    /// Save cache to disk.
    pub fn save_cache(&self) -> Result<(), StabilityError> {
        self.recorder.save()
    }
}

/// Information about a proof test.
#[derive(Debug, Clone)]
pub struct ProofTestInfo {
    /// File path
    pub path: PathBuf,
    /// Proof identifier
    pub proof_id: ProofId,
    /// Proof category
    pub category: ProofCategory,
    /// Extracted SMT formula (if any)
    pub formula: Option<Text>,
    /// File content
    pub content: Text,
}

// Helper functions for parsing test files

/// Extract scope (function name) from test content.
fn extract_scope(content: &str) -> Option<Text> {
    static FN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"fn\s+(\w+)").unwrap());

    FN_RE.captures(content).map(|c| c[1].to_string().into())
}

/// Find the first assert line number.
fn find_first_assert_line(content: &str) -> Option<usize> {
    for (i, line) in content.lines().enumerate() {
        if line.contains("assert") || line.contains("requires") || line.contains("ensures") {
            return Some(i + 1);
        }
    }
    None
}

/// Extract description from test comment.
fn extract_description(content: &str) -> Option<Text> {
    static DESC_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"///\s*(.+)").unwrap());

    DESC_RE.captures(content).map(|c| c[1].to_string().into())
}

/// Extract proof category from tags.
fn extract_category(content: &str) -> Option<ProofCategory> {
    static TAGS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@tags:\s*(.+)").unwrap());

    if let Some(caps) = TAGS_RE.captures(content) {
        let tags = caps[1].to_lowercase();
        if tags.contains("arithmetic") || tags.contains("arith") {
            return Some(ProofCategory::Arithmetic);
        }
        if tags.contains("quantifier") || tags.contains("forall") || tags.contains("exists") {
            return Some(ProofCategory::Quantifier);
        }
        if tags.contains("array") || tags.contains("memory") {
            return Some(ProofCategory::Array);
        }
        if tags.contains("recursive") || tags.contains("termination") {
            return Some(ProofCategory::Recursive);
        }
        if tags.contains("bitvector") || tags.contains("bv") {
            return Some(ProofCategory::BitVector);
        }
        if tags.contains("string") {
            return Some(ProofCategory::String);
        }
    }

    None
}

/// Extract embedded SMT formula if present.
fn extract_smt_formula(content: &str) -> Option<Text> {
    static SMT_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?s)// @smt-formula:\s*```\s*(.+?)\s*```").unwrap());

    SMT_RE.captures(content).map(|c| c[1].to_string().into())
}

/// Generate a mock SMT formula for testing.
fn generate_mock_formula(_content: &str, category: &ProofCategory) -> Text {
    match category {
        ProofCategory::Arithmetic => r#"; Arithmetic proof
(declare-const x Int)
(declare-const y Int)
(assert (> x 0))
(assert (> y 0))
(assert (not (> (+ x y) 0)))
(check-sat)
"#
        .to_string()
        .into(),
        ProofCategory::Quantifier => r#"; Quantifier proof
(declare-sort T 0)
(declare-fun P (T) Bool)
(assert (forall ((x T)) (P x)))
(assert (exists ((y T)) (not (P y))))
(check-sat)
"#
        .to_string()
        .into(),
        ProofCategory::Array => r#"; Array proof
(declare-const arr (Array Int Int))
(declare-const i Int)
(declare-const v Int)
(assert (= (select (store arr i v) i) v))
(check-sat)
"#
        .to_string()
        .into(),
        ProofCategory::Recursive => r#"; Recursive proof (well-foundedness)
(declare-fun f (Int) Int)
(assert (forall ((n Int)) (=> (> n 0) (< (f n) n))))
(check-sat)
"#
        .to_string()
        .into(),
        _ => r#"; Generic proof
(declare-const x Int)
(assert (> x 0))
(assert (<= x 0))
(check-sat)
"#
        .to_string()
        .into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_scope() {
        let content = "fn main() { }";
        assert_eq!(extract_scope(content), Some("main".to_string().into()));

        let content = "fn calculate_sum(x: Int) -> Int { }";
        assert_eq!(extract_scope(content), Some("calculate_sum".to_string().into()));
    }

    #[test]
    fn test_extract_category() {
        let content = "// @tags: arithmetic, simple";
        assert_eq!(extract_category(content), Some(ProofCategory::Arithmetic));

        let content = "// @tags: quantifier, forall";
        assert_eq!(extract_category(content), Some(ProofCategory::Quantifier));
    }

    #[test]
    fn test_mock_formula_generation() {
        let formula = generate_mock_formula("", &ProofCategory::Arithmetic);
        assert!(formula.contains("declare-const x Int"));
        assert!(formula.contains("check-sat"));
    }
}
