//! Configuration file support for VCS test runner.
//!
//! Supports `.vtest.toml` configuration files for project-wide test settings.
//!
//! # Configuration File Format
//!
//! ```toml
//! # .vtest.toml - VCS Test Runner Configuration
//!
//! [runner]
//! # Number of parallel jobs (default: number of CPU cores)
//! jobs = 4
//! # Default timeout in milliseconds
//! timeout = 30000
//! # Default progress style: simple, bar, verbose, quiet, streaming
//! progress = "bar"
//! # Whether to use colors
//! colors = true
//! # Whether to fail fast on first error
//! fail_fast = false
//!
//! [paths]
//! # Path to verum interpreter
//! interpreter = "verum-interpreter"
//! # Path to JIT compiler
//! jit = "verum-jit"
//! # Path to AOT compiler
//! aot = "verum-aot"
//! # Working directory for tests
//! work_dir = "."
//!
//! [report]
//! # Report formats to generate
//! formats = ["console", "json"]
//! # Output directory for reports
//! output_dir = "test-results"
//! # Show diffs for failures
//! show_diff = true
//! # Diff context lines
//! diff_context = 3
//!
//! [filter]
//! # Default level filter (L0, L1, L2, L3, L4, or "all")
//! level = "all"
//! # Default tier filter
//! tiers = [0, 1, 2, 3]
//! # Tags to include
//! include_tags = []
//! # Tags to exclude
//! exclude_tags = ["slow", "flaky"]
//!
//! [watch]
//! # Debounce interval in milliseconds
//! debounce = 200
//! # Directories to watch
//! watch_dirs = ["vcs/specs"]
//! # File patterns to watch
//! patterns = ["*.vr"]
//!
//! [features]
//! # Available feature flags for @requires directive
//! available = ["basic", "std"]
//!
//! [env]
//! # Environment variables for test execution
//! VERUM_DEBUG = "1"
//! VERUM_FEATURES = "basic,std"
//! ```
//!
//! # Configuration Resolution
//!
//! Configuration is resolved in order (later overrides earlier):
//! 1. Built-in defaults
//! 2. Global config `~/.config/vtest/config.toml`
//! 3. Project config `.vtest.toml` (searched up from current directory)
//! 4. Environment variables (VTEST_*)
//! 5. Command-line arguments

use crate::directive::{Level, Tier};
use crate::executor::ExecutorConfig;
use crate::progress::{ProgressConfig, ProgressStyle};
use crate::report::ReportFormat;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use verum_common::{List, Map, Set, Text};

/// Error type for configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse config file: {0}")]
    ParseError(#[from] toml::de::Error),

    #[error("Invalid configuration: {0}")]
    ValidationError(Text),

    #[error("Config file not found: {0}")]
    NotFound(Text),
}

/// Full VCS test runner configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VTestConfig {
    /// Runner settings
    pub runner: RunnerConfig,
    /// Path settings
    pub paths: PathsConfig,
    /// Report settings
    pub report: ReportConfig,
    /// Filter settings
    pub filter: FilterConfig,
    /// Watch mode settings
    pub watch: WatchConfig,
    /// Feature flags
    pub features: FeaturesConfig,
    /// Environment variables
    pub env: HashMap<String, String>,
}

impl Default for VTestConfig {
    fn default() -> Self {
        Self {
            runner: RunnerConfig::default(),
            paths: PathsConfig::default(),
            report: ReportConfig::default(),
            filter: FilterConfig::default(),
            watch: WatchConfig::default(),
            features: FeaturesConfig::default(),
            env: HashMap::new(),
        }
    }
}

/// Runner configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunnerConfig {
    /// Number of parallel jobs
    pub jobs: Option<usize>,
    /// Default timeout in milliseconds
    pub timeout: u64,
    /// Progress display style
    pub progress: String,
    /// Use colors in output
    pub colors: bool,
    /// Fail fast on first error
    pub fail_fast: bool,
    /// Verbose output
    pub verbose: bool,
    /// Retry failed tests
    pub retry: u32,
    /// Retry delay in milliseconds
    pub retry_delay: u64,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            jobs: None, // Use number of CPUs
            timeout: 30_000,
            progress: "bar".to_string(),
            colors: true,
            fail_fast: false,
            verbose: false,
            retry: 0,
            retry_delay: 1000,
        }
    }
}

/// Path configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    /// Path to verum interpreter
    pub interpreter: PathBuf,
    /// Path to JIT compiler (baseline)
    pub jit_base: PathBuf,
    /// Path to JIT compiler (optimized)
    pub jit_opt: PathBuf,
    /// Path to AOT compiler
    pub aot: PathBuf,
    /// Working directory
    pub work_dir: PathBuf,
    /// Test directories
    pub test_dirs: List<PathBuf>,
}

impl Default for PathsConfig {
    fn default() -> Self {
        // Try to find binaries in vcs/bin relative to current directory
        // This allows vtest to work from the project root
        let bin_dir = Self::find_bin_dir();

        Self {
            interpreter: bin_dir.join("verum-interpreter"),
            jit_base: bin_dir.join("verum-jit"),
            jit_opt: bin_dir.join("verum-jit"),
            aot: bin_dir.join("verum-aot"),
            work_dir: PathBuf::from("."),
            test_dirs: vec![PathBuf::from("vcs/specs")].into(),
        }
    }
}

impl PathsConfig {
    /// Find the vcs/bin directory by searching up from the current directory.
    fn find_bin_dir() -> PathBuf {
        // First try relative to current working directory
        let cwd_bin = PathBuf::from("vcs/bin");
        if cwd_bin.exists() {
            return cwd_bin;
        }

        // Try to find by searching up the directory tree
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors() {
                let bin_dir = ancestor.join("vcs").join("bin");
                if bin_dir.exists() {
                    return bin_dir;
                }
            }
        }

        // Fall back to system PATH lookup
        PathBuf::new()
    }
}

/// Report configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportConfig {
    /// Report formats to generate
    pub formats: List<String>,
    /// Output directory for reports
    pub output_dir: PathBuf,
    /// Show diffs for failures
    pub show_diff: bool,
    /// Diff context lines
    pub diff_context: usize,
    /// Include passed tests in report
    pub include_passed: bool,
    /// Include skipped tests in report
    pub include_skipped: bool,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            formats: vec!["console".to_string()].into(),
            output_dir: PathBuf::from("test-results"),
            show_diff: true,
            diff_context: 3,
            include_passed: true,
            include_skipped: true,
        }
    }
}

/// Filter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    /// Level filter
    pub level: String,
    /// Tier filter
    pub tiers: List<u8>,
    /// Tags to include
    pub include_tags: List<String>,
    /// Tags to exclude
    pub exclude_tags: List<String>,
    /// Glob patterns for test files
    pub patterns: List<String>,
    /// Glob patterns to exclude
    pub exclude_patterns: List<String>,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            level: "all".to_string(),
            tiers: vec![0, 1, 2, 3].into(),
            include_tags: List::new(),
            exclude_tags: List::new(),
            patterns: vec!["**/*.vr".to_string()].into(),
            exclude_patterns: List::new(),
        }
    }
}

/// Watch mode configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchConfig {
    /// Debounce interval in milliseconds
    pub debounce: u64,
    /// Directories to watch
    pub watch_dirs: List<PathBuf>,
    /// File patterns to watch
    pub patterns: List<String>,
    /// Clear screen on rerun
    pub clear: bool,
    /// Run on start
    pub run_on_start: bool,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: 200,
            watch_dirs: vec![PathBuf::from("vcs/specs")].into(),
            patterns: vec!["*.vr".to_string()].into(),
            clear: true,
            run_on_start: true,
        }
    }
}

/// Feature flags configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FeaturesConfig {
    /// Available feature flags
    pub available: List<String>,
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            available: vec![
                "basic".to_string(),
                "std".to_string(),
                "bounds-checking".to_string(),
                "panic-handling".to_string(),
                "cbgr-runtime".to_string(),
            ]
            .into(),
        }
    }
}

impl VTestConfig {
    /// Load configuration from a file.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    /// Load configuration from default locations.
    ///
    /// Searches for `.vtest.toml` in current directory and parents,
    /// then merges with global config if present.
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Self::default();

        // Try global config
        if let Some(home) = dirs::home_dir() {
            let global_config = home.join(".config").join("vtest").join("config.toml");
            if global_config.exists() {
                if let Ok(global) = Self::from_file(&global_config) {
                    config = config.merge(global);
                }
            }
        }

        // Try project config (search up from current directory)
        if let Ok(cwd) = std::env::current_dir() {
            for dir in cwd.ancestors() {
                let project_config = dir.join(".vtest.toml");
                if project_config.exists() {
                    if let Ok(project) = Self::from_file(&project_config) {
                        config = config.merge(project);
                    }
                    break;
                }
            }
        }

        // Apply environment variables
        config = config.apply_env();

        Ok(config)
    }

    /// Merge another config into this one (other takes precedence).
    pub fn merge(mut self, other: Self) -> Self {
        // Runner config
        if other.runner.jobs.is_some() {
            self.runner.jobs = other.runner.jobs;
        }
        self.runner.timeout = other.runner.timeout;
        self.runner.progress = other.runner.progress;
        self.runner.colors = other.runner.colors;
        self.runner.fail_fast = other.runner.fail_fast;
        self.runner.verbose = other.runner.verbose;
        self.runner.retry = other.runner.retry;

        // Paths config
        self.paths = other.paths;

        // Report config
        self.report = other.report;

        // Filter config
        self.filter = other.filter;

        // Watch config
        self.watch = other.watch;

        // Features config
        self.features = other.features;

        // Merge env (other takes precedence)
        for (k, v) in other.env {
            self.env.insert(k, v);
        }

        self
    }

    /// Apply environment variable overrides.
    fn apply_env(mut self) -> Self {
        if let Ok(jobs) = std::env::var("VTEST_JOBS") {
            if let Ok(n) = jobs.parse() {
                self.runner.jobs = Some(n);
            }
        }

        if let Ok(timeout) = std::env::var("VTEST_TIMEOUT") {
            if let Ok(t) = timeout.parse() {
                self.runner.timeout = t;
            }
        }

        if let Ok(progress) = std::env::var("VTEST_PROGRESS") {
            self.runner.progress = progress;
        }

        if std::env::var("VTEST_NO_COLOR").is_ok() || std::env::var("NO_COLOR").is_ok() {
            self.runner.colors = false;
        }

        if std::env::var("VTEST_VERBOSE").is_ok() {
            self.runner.verbose = true;
        }

        if std::env::var("VTEST_FAIL_FAST").is_ok() {
            self.runner.fail_fast = true;
        }

        if let Ok(level) = std::env::var("VTEST_LEVEL") {
            self.filter.level = level;
        }

        self
    }

    /// Validate the configuration.
    fn validate(&self) -> Result<(), ConfigError> {
        // Validate progress style
        if ProgressStyle::from_str(&self.runner.progress).is_none() {
            return Err(ConfigError::ValidationError(format!(
                "Invalid progress style: {}. Valid options: simple, bar, verbose, quiet, streaming",
                self.runner.progress
            ).into()));
        }

        // Validate level
        let level = self.filter.level.to_uppercase();
        if level != "ALL" && Level::from_str(&level).is_err() {
            return Err(ConfigError::ValidationError(format!(
                "Invalid level: {}. Valid options: L0, L1, L2, L3, L4, all",
                self.filter.level
            ).into()));
        }

        // Validate tiers
        for tier in &self.filter.tiers {
            if *tier > 3 {
                return Err(ConfigError::ValidationError(format!(
                    "Invalid tier: {}. Valid options: 0, 1, 2, 3",
                    tier
                ).into()));
            }
        }

        // Validate report formats
        for format in &self.report.formats {
            if ReportFormat::from_str(format).is_none() {
                return Err(ConfigError::ValidationError(format!(
                    "Invalid report format: {}. Valid options: console, json, html, junit, tap, markdown",
                    format
                ).into()));
            }
        }

        Ok(())
    }

    /// Convert to executor config.
    pub fn to_executor_config(&self) -> ExecutorConfig {
        let mut env: Map<Text, Text> = Map::new();
        for (k, v) in &self.env {
            env.insert(k.clone().into(), v.clone().into());
        }

        // Convert available features to Set<Text>
        let mut available_features = verum_common::Set::new();
        for feature in &self.features.available {
            available_features.insert(verum_common::Text::from(feature.as_str()));
        }

        ExecutorConfig {
            interpreter_path: self.paths.interpreter.clone(),
            jit_base_path: self.paths.jit_base.clone(),
            jit_opt_path: self.paths.jit_opt.clone(),
            aot_path: self.paths.aot.clone(),
            work_dir: self.paths.work_dir.clone(),
            default_timeout_ms: self.runner.timeout,
            env,
            use_direct_integration: true,
            verum_cli_path: None,
            available_features,
            compile_time_only: false,
            vbc_output_dir: None,
            vbc_preserve_paths: false,
        }
    }

    /// Convert to progress config.
    pub fn to_progress_config(&self) -> ProgressConfig {
        ProgressConfig {
            style: ProgressStyle::from_str(&self.runner.progress).unwrap_or(ProgressStyle::Bar),
            use_colors: self.runner.colors,
            show_timing: true,
            ..ProgressConfig::default()
        }
    }

    /// Get report formats.
    pub fn report_formats(&self) -> List<ReportFormat> {
        self.report
            .formats
            .iter()
            .filter_map(|s| ReportFormat::from_str(s))
            .collect()
    }

    /// Get level filter.
    pub fn level_filter(&self) -> Option<Level> {
        if self.filter.level.to_uppercase() == "ALL" {
            None
        } else {
            Level::from_str(&self.filter.level).ok()
        }
    }

    /// Get tier filter.
    pub fn tier_filter(&self) -> List<Tier> {
        self.filter
            .tiers
            .iter()
            .filter_map(|t| Tier::from_str(&t.to_string()).ok())
            .collect()
    }

    /// Get include tags as set.
    pub fn include_tags(&self) -> Set<Text> {
        self.filter.include_tags.iter().map(|s| s.clone().into()).collect()
    }

    /// Get exclude tags as set.
    pub fn exclude_tags(&self) -> Set<Text> {
        self.filter.exclude_tags.iter().map(|s| s.clone().into()).collect()
    }

    /// Get number of jobs.
    pub fn jobs(&self) -> usize {
        self.runner.jobs.unwrap_or_else(num_cpus::get)
    }
}

/// Write a default config file.
pub fn write_default_config(path: &Path) -> Result<(), ConfigError> {
    let config = VTestConfig::default();
    let content =
        toml::to_string_pretty(&config).map_err(|e| ConfigError::ValidationError(e.to_string().into()))?;

    // Add helpful comments
    let content = format!(
        r#"# VCS Test Runner Configuration
# See documentation for full options

{}
"#,
        content
    );

    std::fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VTestConfig::default();
        assert_eq!(config.runner.timeout, 30_000);
        assert_eq!(config.runner.progress, "bar");
        assert!(config.runner.colors);
        assert!(!config.runner.fail_fast);
    }

    #[test]
    fn test_config_validation() {
        let mut config = VTestConfig::default();
        assert!(config.validate().is_ok());

        config.runner.progress = "invalid".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_from_toml() {
        let toml = r#"
            [runner]
            timeout = 60000
            progress = "verbose"
            colors = false

            [filter]
            level = "L0"
            tiers = [0, 1]
        "#;

        let config: VTestConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.runner.timeout, 60000);
        assert_eq!(config.runner.progress, "verbose");
        assert!(!config.runner.colors);
        assert_eq!(config.filter.level, "L0");
        let expected_tiers: List<u8> = vec![0, 1].into();
        assert_eq!(config.filter.tiers, expected_tiers);
    }

    #[test]
    fn test_config_merge() {
        let base = VTestConfig::default();
        let mut override_config = VTestConfig::default();
        override_config.runner.timeout = 60000;
        override_config.runner.fail_fast = true;

        let merged = base.merge(override_config);
        assert_eq!(merged.runner.timeout, 60000);
        assert!(merged.runner.fail_fast);
    }

    #[test]
    fn test_executor_config_conversion() {
        let config = VTestConfig::default();
        let exec_config = config.to_executor_config();
        assert_eq!(exec_config.default_timeout_ms, 30_000);
    }

    #[test]
    fn test_progress_config_conversion() {
        let mut config = VTestConfig::default();
        config.runner.progress = "verbose".to_string();
        config.runner.colors = false;

        let progress_config = config.to_progress_config();
        assert_eq!(progress_config.style, ProgressStyle::Verbose);
        assert!(!progress_config.use_colors);
    }

    #[test]
    fn test_level_filter() {
        let mut config = VTestConfig::default();

        config.filter.level = "all".to_string();
        assert!(config.level_filter().is_none());

        config.filter.level = "L0".to_string();
        assert_eq!(config.level_filter(), Some(Level::L0));

        config.filter.level = "L2".to_string();
        assert_eq!(config.level_filter(), Some(Level::L2));
    }

    #[test]
    fn test_tier_filter() {
        let mut config = VTestConfig::default();
        config.filter.tiers = vec![0, 2].into();

        let tiers = config.tier_filter();
        assert_eq!(tiers.len(), 2);
        assert!(tiers.contains(&Tier::Tier0));
        assert!(tiers.contains(&Tier::Tier2));
    }
}
