//! Configuration for proof stability testing.
//!
//! This module provides configuration types for controlling:
//! - SMT solver timeouts and random seeds
//! - Proof cache location and behavior
//! - Stability thresholds for flakiness detection

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use verum_common::{List, Map, Text};

/// Main configuration for proof stability testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityConfig {
    /// Solver configuration
    pub solver: SolverConfig,
    /// Cache configuration
    pub cache: CacheConfig,
    /// Stability thresholds
    pub thresholds: StabilityThresholds,
    /// Execution configuration
    pub execution: ExecutionConfig,
    /// Reporting configuration
    pub reporting: ReportingConfig,
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            solver: SolverConfig::default(),
            cache: CacheConfig::default(),
            thresholds: StabilityThresholds::default(),
            execution: ExecutionConfig::default(),
            reporting: ReportingConfig::default(),
        }
    }
}

impl StabilityConfig {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &std::path::Path) -> Result<Self, crate::StabilityError> {
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content).map_err(|e| crate::StabilityError::ConfigError(e.to_string().into()))
    }

    /// Load configuration from default locations.
    pub fn load_default() -> Result<Self, crate::StabilityError> {
        let paths = [
            "proof_stability.toml",
            ".proof_stability.toml",
            "vcs/proof_stability.toml",
            ".vcs/proof_stability.toml",
        ];

        for path in paths {
            if std::path::Path::new(path).exists() {
                return Self::from_file(std::path::Path::new(path));
            }
        }

        Ok(Self::default())
    }
}

/// SMT solver configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverConfig {
    /// Default solver to use (z3, cvc5, etc.)
    pub default_solver: Text,
    /// Path to Z3 binary
    pub z3_path: Option<PathBuf>,
    /// Path to CVC5 binary
    pub cvc5_path: Option<PathBuf>,
    /// Default timeout in milliseconds
    pub default_timeout_ms: u64,
    /// Maximum timeout in milliseconds
    pub max_timeout_ms: u64,
    /// Random seed for deterministic solving
    pub random_seed: Option<u64>,
    /// Enable deterministic mode
    pub deterministic: bool,
    /// Number of retries with different seeds
    pub retry_count: usize,
    /// Seeds to use for retries (if not random)
    pub retry_seeds: List<u64>,
    /// Solver-specific options
    pub options: Map<Text, Text>,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            default_solver: "z3".to_string().into(),
            z3_path: None,
            cvc5_path: None,
            default_timeout_ms: 30_000,
            max_timeout_ms: 300_000,
            random_seed: Some(42), // Deterministic by default
            deterministic: true,
            retry_count: 3,
            retry_seeds: vec![42, 123, 456].into(),
            options: Map::new(),
        }
    }
}

impl SolverConfig {
    /// Get the solver path for a given solver name.
    pub fn solver_path(&self, solver: &str) -> Option<PathBuf> {
        match solver.to_lowercase().as_str() {
            "z3" => self.z3_path.clone().or_else(|| Some(PathBuf::from("z3"))),
            "cvc5" => self
                .cvc5_path
                .clone()
                .or_else(|| Some(PathBuf::from("cvc5"))),
            _ => None,
        }
    }

    /// Get effective random seed.
    pub fn effective_seed(&self) -> u64 {
        self.random_seed.unwrap_or_else(|| {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64
        })
    }

    /// Get seeds for stability testing (multiple runs).
    pub fn stability_seeds(&self) -> List<u64> {
        if !self.retry_seeds.is_empty() {
            self.retry_seeds.clone()
        } else {
            let base_seed = self.effective_seed();
            (0..self.retry_count as u64)
                .map(|i| base_seed.wrapping_add(i * 12345))
                .collect()
        }
    }
}

/// Proof cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Enable proof caching
    pub enabled: bool,
    /// Cache directory path
    pub cache_dir: PathBuf,
    /// Maximum cache size in MB
    pub max_size_mb: u64,
    /// Cache TTL in seconds (0 = infinite)
    pub ttl_seconds: u64,
    /// Store proof artifacts (SMT-LIB files)
    pub store_artifacts: bool,
    /// Store counterexamples
    pub store_counterexamples: bool,
    /// Compression level (0 = none, 9 = max)
    pub compression_level: u8,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_dir: PathBuf::from(".vcs/proof_cache"),
            max_size_mb: 1024,
            ttl_seconds: 0, // No expiration
            store_artifacts: true,
            store_counterexamples: true,
            compression_level: 6,
        }
    }
}

/// Stability thresholds for flakiness detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityThresholds {
    /// Minimum number of runs for stability determination
    pub min_runs: usize,
    /// Threshold for considering a proof stable (0.0-1.0)
    pub stable_threshold: f64,
    /// Threshold for considering a proof flaky (0.0-1.0)
    pub flaky_threshold: f64,
    /// Maximum variance in timing for stable proofs (coefficient of variation)
    pub timing_variance_threshold: f64,
    /// Threshold for timeout instability
    pub timeout_variance_threshold: f64,
    /// Per-category thresholds (optional overrides)
    pub category_thresholds: Map<Text, f64>,
}

impl Default for StabilityThresholds {
    fn default() -> Self {
        let mut category_thresholds = Map::new();
        category_thresholds.insert("arithmetic".to_string().into(), 0.99);
        category_thresholds.insert("quantifier".to_string().into(), 0.80);
        category_thresholds.insert("array".to_string().into(), 0.95);
        category_thresholds.insert("recursive".to_string().into(), 0.85);
        category_thresholds.insert("bitvector".to_string().into(), 0.98);

        Self {
            min_runs: 3,
            stable_threshold: 0.95,
            flaky_threshold: 0.80,
            timing_variance_threshold: 0.50, // 50% CV
            timeout_variance_threshold: 0.30,
            category_thresholds,
        }
    }
}

impl StabilityThresholds {
    /// Get threshold for a specific category.
    pub fn threshold_for_category(&self, category: &crate::ProofCategory) -> f64 {
        let key: Text = category.to_string().into();
        self.category_thresholds
            .get(&key)
            .copied()
            .unwrap_or(self.stable_threshold)
    }
}

/// Execution configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Number of parallel proof attempts
    pub parallel: usize,
    /// Default number of stability runs per proof
    pub stability_runs: usize,
    /// Continue on individual proof failure
    pub continue_on_failure: bool,
    /// Fail fast on first flaky proof
    pub fail_fast_flaky: bool,
    /// Record all attempts (even successful stable ones)
    pub record_all: bool,
    /// Test paths to include
    pub test_paths: List<PathBuf>,
    /// Glob pattern for test files
    pub test_pattern: Text,
    /// Patterns to exclude
    pub exclude_patterns: List<Text>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            parallel: num_cpus::get(),
            stability_runs: 5,
            continue_on_failure: true,
            fail_fast_flaky: false,
            record_all: false,
            test_paths: vec![PathBuf::from("specs")].into(),
            test_pattern: "**/*.vr".to_string().into(),
            exclude_patterns: vec!["**/skip/**".to_string().into(), "**/wip/**".to_string().into()].into(),
        }
    }
}

/// Reporting configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingConfig {
    /// Output format (console, json, html, markdown)
    pub format: Text,
    /// Output path (None = stdout)
    pub output_path: Option<PathBuf>,
    /// Show detailed flaky proof information
    pub show_flaky_details: bool,
    /// Show timing statistics
    pub show_timing: bool,
    /// Show solver-specific details
    pub show_solver_details: bool,
    /// Generate regression report
    pub generate_regression_report: bool,
    /// Regression baseline path
    pub baseline_path: Option<PathBuf>,
    /// Use colors in console output
    pub use_colors: bool,
    /// Verbose output
    pub verbose: bool,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            format: "console".to_string().into(),
            output_path: None,
            show_flaky_details: true,
            show_timing: true,
            show_solver_details: false,
            generate_regression_report: true,
            baseline_path: None,
            use_colors: true,
            verbose: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = StabilityConfig::default();
        assert!(config.solver.deterministic);
        assert_eq!(config.solver.retry_count, 3);
        assert!(config.cache.enabled);
    }

    #[test]
    fn test_solver_config_seeds() {
        let config = SolverConfig {
            retry_seeds: vec![1, 2, 3].into(),
            ..Default::default()
        };
        let seeds = config.stability_seeds();
        let expected: List<u64> = vec![1, 2, 3].into();
        assert_eq!(seeds, expected);
    }

    #[test]
    fn test_threshold_for_category() {
        let thresholds = StabilityThresholds::default();
        assert!(thresholds.threshold_for_category(&crate::ProofCategory::Arithmetic) > 0.95);
        assert!(thresholds.threshold_for_category(&crate::ProofCategory::Quantifier) < 0.90);
    }

    #[test]
    fn test_config_serialization() {
        let config = StabilityConfig::default();
        let toml = toml::to_string_pretty(&config).unwrap();
        let parsed: StabilityConfig = toml::from_str(&toml).unwrap();
        assert_eq!(config.solver.default_solver, parsed.solver.default_solver);
    }
}
