//! SMT Configuration System - Unified Configuration for All Backends
//!
//! This module provides a comprehensive configuration system for SMT backends
//! with support for:
//! - Backend selection (Z3, CVC5, auto, portfolio)
//! - Fallback strategies
//! - Portfolio solving modes
//! - Cross-validation
//! - Environment variables
//! - TOML/JSON configuration files
//!
//! ## Configuration Hierarchy
//!
//! 1. Default values (hardcoded)
//! 2. Configuration file (TOML/JSON)
//! 3. Environment variables (override file)
//! 4. Programmatic API (override all)
//!
//! ## Example Configuration File (TOML)
//!
//! ```toml
//! [smt]
//! backend = "auto"
//! timeout_ms = 30000
//! verbose = false
//!
//! [smt.fallback]
//! enabled = true
//! on_timeout = true
//! on_unknown = true
//! on_error = true
//! max_attempts = 2
//!
//! [smt.portfolio]
//! enabled = false
//! mode = "first"
//! max_threads = 2
//! timeout_per_solver = 30000
//!
//! [smt.validation]
//! enabled = false
//! cross_validate = false
//! fail_on_mismatch = false
//! log_mismatches = true
//! ```
//!
//! Configuration for SMT-based refinement type verification: solver selection,
//! timeouts, caching, and strategy parameters for `@verify(proof)` compilation.

use serde::{Deserialize, Serialize};
use std::path::Path;

use verum_common::Maybe;

#[cfg(feature = "cvc5")]
use crate::backend_switcher::{
    BackendChoice, FallbackConfig, PortfolioConfig, PortfolioMode, SwitcherConfig, ValidationConfig,
};

// ==================== Main Configuration ====================

/// Complete SMT configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtConfig {
    /// Primary backend selection
    #[serde(default = "default_backend")]
    pub backend: BackendChoice,

    /// Global timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,

    /// Enable verbose logging
    #[serde(default)]
    pub verbose: bool,

    /// Fallback configuration
    #[serde(default)]
    pub fallback: FallbackConfig,

    /// Portfolio configuration
    #[serde(default)]
    pub portfolio: PortfolioConfig,

    /// Validation configuration
    #[serde(default)]
    pub validation: ValidationConfig,

    /// Backend-specific configurations
    #[serde(default)]
    pub z3: Z3Config,

    #[serde(default)]
    pub cvc5: Cvc5Config,
}

impl Default for SmtConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            timeout_ms: default_timeout(),
            verbose: false,
            fallback: FallbackConfig::default(),
            portfolio: PortfolioConfig::default(),
            validation: ValidationConfig::default(),
            z3: Z3Config::default(),
            cvc5: Cvc5Config::default(),
        }
    }
}

fn default_backend() -> BackendChoice {
    BackendChoice::Auto
}

fn default_timeout() -> u64 {
    30000 // 30 seconds
}

// ==================== Z3-Specific Configuration ====================

/// Z3-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Z3Config {
    /// Enable proof generation
    #[serde(default = "default_true")]
    pub enable_proofs: bool,

    /// Enable unsat core minimization
    #[serde(default = "default_true")]
    pub minimize_cores: bool,

    /// Enable model-based quantifier instantiation
    #[serde(default = "default_true")]
    pub enable_mbqi: bool,

    /// Enable pattern-based quantifier instantiation
    #[serde(default = "default_true")]
    pub enable_patterns: bool,

    /// Random seed for reproducibility
    pub random_seed: Maybe<u32>,

    /// Memory limit in megabytes
    pub memory_limit_mb: Maybe<usize>,

    /// Number of parallel workers
    #[serde(default = "default_num_workers")]
    pub num_workers: usize,

    /// Enable automatic tactic selection
    #[serde(default = "default_true")]
    pub auto_tactics: bool,
}

impl Default for Z3Config {
    fn default() -> Self {
        Self {
            enable_proofs: true,
            minimize_cores: true,
            enable_mbqi: true,
            enable_patterns: true,
            random_seed: Maybe::None,
            memory_limit_mb: Maybe::Some(8192), // 8GB
            num_workers: num_cpus::get().max(4),
            auto_tactics: true,
        }
    }
}

// ==================== CVC5-Specific Configuration ====================

/// CVC5-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cvc5Config {
    /// SMT-LIB logic
    #[serde(default = "default_logic")]
    pub logic: String,

    /// Enable incremental solving
    #[serde(default = "default_true")]
    pub incremental: bool,

    /// Produce models for SAT results
    #[serde(default = "default_true")]
    pub produce_models: bool,

    /// Produce proofs for UNSAT results
    #[serde(default = "default_true")]
    pub produce_proofs: bool,

    /// Produce unsat cores
    #[serde(default = "default_true")]
    pub produce_unsat_cores: bool,

    /// Enable preprocessing
    #[serde(default = "default_true")]
    pub preprocessing: bool,

    /// Quantifier instantiation mode
    #[serde(default = "default_quantifier_mode")]
    pub quantifier_mode: String,

    /// Random seed for reproducibility
    pub random_seed: Maybe<u32>,

    /// Verbosity level (0-5)
    #[serde(default)]
    pub verbosity: u32,
}

impl Default for Cvc5Config {
    fn default() -> Self {
        Self {
            logic: default_logic(),
            incremental: true,
            produce_models: true,
            produce_proofs: true,
            produce_unsat_cores: true,
            preprocessing: true,
            quantifier_mode: default_quantifier_mode(),
            random_seed: Maybe::None,
            verbosity: 0,
        }
    }
}

fn default_logic() -> String {
    "ALL".to_string()
}

fn default_quantifier_mode() -> String {
    "auto".to_string()
}

fn default_true() -> bool {
    true
}

fn default_num_workers() -> usize {
    num_cpus::get().max(4)
}

// ==================== Configuration Loading ====================

impl SmtConfig {
    /// Create configuration from environment variables
    ///
    /// Environment variables (all optional):
    /// - `VERUM_SMT_BACKEND`: Backend choice (z3, cvc5, auto, portfolio)
    /// - `VERUM_SMT_TIMEOUT`: Timeout in milliseconds
    /// - `VERUM_SMT_VERBOSE`: Enable verbose logging (true/false)
    /// - `VERUM_SMT_FALLBACK`: Enable fallback (true/false)
    /// - `VERUM_SMT_FALLBACK_ON_TIMEOUT`: Fallback on timeout (true/false)
    /// - `VERUM_SMT_FALLBACK_ON_UNKNOWN`: Fallback on unknown (true/false)
    /// - `VERUM_SMT_FALLBACK_ON_ERROR`: Fallback on error (true/false)
    /// - `VERUM_SMT_PORTFOLIO_MODE`: Portfolio mode (first, consensus, vote)
    /// - `VERUM_SMT_PORTFOLIO_THREADS`: Number of threads for portfolio
    /// - `VERUM_SMT_CROSS_VALIDATE`: Enable cross-validation (true/false)
    /// - `VERUM_SMT_Z3_PROOFS`: Enable Z3 proofs (true/false)
    /// - `VERUM_SMT_Z3_TACTICS`: Enable Z3 auto-tactics (true/false)
    /// - `VERUM_SMT_CVC5_LOGIC`: CVC5 logic (QF_LIA, etc.)
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Backend selection
        if let Ok(backend_str) = std::env::var("VERUM_SMT_BACKEND") {
            if let Ok(backend) = backend_str.parse() {
                config.backend = backend;
            }
        }

        // Timeout
        if let Ok(timeout_str) = std::env::var("VERUM_SMT_TIMEOUT") {
            if let Ok(timeout) = timeout_str.parse() {
                config.timeout_ms = timeout;
            }
        }

        // Verbose
        if let Ok(verbose_str) = std::env::var("VERUM_SMT_VERBOSE") {
            config.verbose = verbose_str.parse().unwrap_or(false);
        }

        // Fallback configuration
        if let Ok(fallback_str) = std::env::var("VERUM_SMT_FALLBACK") {
            config.fallback.enabled = fallback_str.parse().unwrap_or(true);
        }

        if let Ok(on_timeout_str) = std::env::var("VERUM_SMT_FALLBACK_ON_TIMEOUT") {
            config.fallback.on_timeout = on_timeout_str.parse().unwrap_or(true);
        }

        if let Ok(on_unknown_str) = std::env::var("VERUM_SMT_FALLBACK_ON_UNKNOWN") {
            config.fallback.on_unknown = on_unknown_str.parse().unwrap_or(true);
        }

        if let Ok(on_error_str) = std::env::var("VERUM_SMT_FALLBACK_ON_ERROR") {
            config.fallback.on_error = on_error_str.parse().unwrap_or(true);
        }

        // Portfolio configuration
        if let Ok(mode_str) = std::env::var("VERUM_SMT_PORTFOLIO_MODE") {
            config.portfolio.mode = match mode_str.to_lowercase().as_str() {
                "first" => PortfolioMode::FirstResult,
                "consensus" => PortfolioMode::Consensus,
                "vote" => PortfolioMode::VoteOnDisagree,
                _ => PortfolioMode::FirstResult,
            };
        }

        if let Ok(threads_str) = std::env::var("VERUM_SMT_PORTFOLIO_THREADS") {
            if let Ok(threads) = threads_str.parse() {
                config.portfolio.max_threads = threads;
            }
        }

        // Validation configuration
        if let Ok(validate_str) = std::env::var("VERUM_SMT_CROSS_VALIDATE") {
            config.validation.cross_validate = validate_str.parse().unwrap_or(false);
        }

        // Z3-specific configuration
        if let Ok(proofs_str) = std::env::var("VERUM_SMT_Z3_PROOFS") {
            config.z3.enable_proofs = proofs_str.parse().unwrap_or(true);
        }

        if let Ok(tactics_str) = std::env::var("VERUM_SMT_Z3_TACTICS") {
            config.z3.auto_tactics = tactics_str.parse().unwrap_or(true);
        }

        // CVC5-specific configuration
        if let Ok(logic_str) = std::env::var("VERUM_SMT_CVC5_LOGIC") {
            config.cvc5.logic = logic_str;
        }

        config
    }

    /// Load configuration from TOML file
    pub fn from_toml_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::IoError(e.to_string()))?;

        toml::from_str(&contents)
            .map_err(|e| ConfigError::ParseError(format!("TOML parse error: {}", e)))
    }

    /// Load configuration from JSON file
    pub fn from_json_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ConfigError::IoError(e.to_string()))?;

        serde_json::from_str(&contents)
            .map_err(|e| ConfigError::ParseError(format!("JSON parse error: {}", e)))
    }

    /// Save configuration to TOML file
    pub fn to_toml_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let toml_str =
            toml::to_string_pretty(self).map_err(|e| ConfigError::SerializeError(e.to_string()))?;

        std::fs::write(path.as_ref(), toml_str).map_err(|e| ConfigError::IoError(e.to_string()))
    }

    /// Save configuration to JSON file
    pub fn to_json_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let json_str = serde_json::to_string_pretty(self)
            .map_err(|e| ConfigError::SerializeError(e.to_string()))?;

        std::fs::write(path.as_ref(), json_str).map_err(|e| ConfigError::IoError(e.to_string()))
    }

    /// Convert to backend switcher configuration
    pub fn to_switcher_config(&self) -> SwitcherConfig {
        SwitcherConfig {
            default_backend: self.backend,
            fallback: self.fallback.clone(),
            portfolio: self.portfolio.clone(),
            validation: self.validation.clone(),
            timeout_ms: self.timeout_ms,
            verbose: self.verbose,
        }
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Check timeout is reasonable
        if self.timeout_ms == 0 {
            return Err(ConfigError::ValidationError(
                "Timeout must be greater than 0".to_string(),
            ));
        }

        if self.timeout_ms > 3600000 {
            // 1 hour
            return Err(ConfigError::ValidationError(
                "Timeout too large (>1 hour)".to_string(),
            ));
        }

        // Check portfolio configuration
        if self.portfolio.enabled && self.portfolio.max_threads == 0 {
            return Err(ConfigError::ValidationError(
                "Portfolio max_threads must be > 0".to_string(),
            ));
        }

        // Check fallback configuration
        if self.fallback.enabled && self.fallback.max_attempts == 0 {
            return Err(ConfigError::ValidationError(
                "Fallback max_attempts must be > 0".to_string(),
            ));
        }

        Ok(())
    }

    /// Apply configuration overrides
    pub fn with_overrides(mut self, overrides: ConfigOverrides) -> Self {
        if let Some(backend) = overrides.backend {
            self.backend = backend;
        }

        if let Some(timeout) = overrides.timeout_ms {
            self.timeout_ms = timeout;
        }

        if let Some(verbose) = overrides.verbose {
            self.verbose = verbose;
        }

        if let Some(fallback_enabled) = overrides.fallback_enabled {
            self.fallback.enabled = fallback_enabled;
        }

        if let Some(portfolio_enabled) = overrides.portfolio_enabled {
            self.portfolio.enabled = portfolio_enabled;
        }

        self
    }
}

// ==================== Configuration Overrides ====================

/// Configuration overrides for programmatic API
#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    pub backend: Option<BackendChoice>,
    pub timeout_ms: Option<u64>,
    pub verbose: Option<bool>,
    pub fallback_enabled: Option<bool>,
    pub portfolio_enabled: Option<bool>,
}

// ==================== Error Types ====================

/// Configuration error
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    IoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Serialize error: {0}")]
    SerializeError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

// ==================== Configuration Presets ====================

impl SmtConfig {
    /// Development preset: fast iteration, minimal checks
    pub fn development() -> Self {
        Self {
            backend: BackendChoice::Z3,
            timeout_ms: 5000, // 5s
            verbose: true,
            fallback: FallbackConfig {
                enabled: false,
                ..Default::default()
            },
            portfolio: PortfolioConfig {
                enabled: false,
                ..Default::default()
            },
            validation: ValidationConfig {
                enabled: false,
                ..Default::default()
            },
            z3: Z3Config {
                enable_proofs: false,
                minimize_cores: false,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Production preset: reliability, fallback, validation
    pub fn production() -> Self {
        Self {
            backend: BackendChoice::Auto,
            timeout_ms: 30000, // 30s
            verbose: false,
            fallback: FallbackConfig {
                enabled: true,
                on_timeout: true,
                on_unknown: true,
                on_error: true,
                max_attempts: 3,
            },
            portfolio: PortfolioConfig {
                enabled: false,
                ..Default::default()
            },
            validation: ValidationConfig {
                enabled: true,
                cross_validate: true,
                fail_on_mismatch: true,
                log_mismatches: true,
            },
            z3: Z3Config {
                enable_proofs: true,
                minimize_cores: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Performance preset: portfolio solving, aggressive parallelism
    pub fn performance() -> Self {
        Self {
            backend: BackendChoice::Portfolio,
            timeout_ms: 30000,
            verbose: false,
            fallback: FallbackConfig {
                enabled: false,
                ..Default::default()
            },
            portfolio: PortfolioConfig {
                enabled: true,
                mode: PortfolioMode::FirstResult,
                max_threads: num_cpus::get(),
                timeout_per_solver: 30000,
                kill_on_first: true,
            },
            validation: ValidationConfig {
                enabled: false,
                ..Default::default()
            },
            z3: Z3Config {
                auto_tactics: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Debugging preset: maximum validation and diagnostics
    pub fn debugging() -> Self {
        Self {
            backend: BackendChoice::Z3,
            timeout_ms: 60000, // 1 minute
            verbose: true,
            fallback: FallbackConfig {
                enabled: false,
                ..Default::default()
            },
            portfolio: PortfolioConfig {
                enabled: false,
                ..Default::default()
            },
            validation: ValidationConfig {
                enabled: true,
                cross_validate: true,
                fail_on_mismatch: true,
                log_mismatches: true,
            },
            z3: Z3Config {
                enable_proofs: true,
                minimize_cores: true,
                ..Default::default()
            },
            cvc5: Cvc5Config {
                verbosity: 3,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

// ==================== Module Statistics ====================

// Total lines: ~530
// Complete configuration system
// Features:
// - TOML/JSON file support
// - Environment variable overrides
// - Validation
// - Presets (development, production, performance, debugging)
// - Backend-specific configurations
// - Comprehensive documentation
