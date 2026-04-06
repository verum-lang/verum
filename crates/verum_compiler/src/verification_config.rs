//! Verification configuration from verum.toml
//!
//! Verification configuration: controls SMT timeout, budget, slow-verification
//! thresholds, and hot-path detection parameters for the verification pipeline.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use verum_common::Text;

/// Verification configuration from verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Verification settings
    #[serde(default)]
    pub verify: VerifySection,
}

/// [verify] section in verum.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifySection {
    /// Total budget for verification across entire project (e.g., "120s")
    #[serde(default)]
    pub total_budget: Option<String>,

    /// Per-function warning threshold (e.g., "5s")
    #[serde(default = "default_slow_threshold")]
    pub slow_threshold: String,

    /// Cache directory path
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,

    /// Maximum cache size (e.g., "500MB")
    #[serde(default = "default_cache_max_size")]
    pub cache_max_size: String,

    /// Cache time-to-live (e.g., "30d")
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl: String,

    /// Distributed cache URL (e.g., "s3://my-bucket/verum-cache")
    #[serde(default)]
    pub distributed_cache: Option<String>,

    /// Distributed cache trust level ("all", "signatures", or "signatures_and_expiry")
    #[serde(default = "default_distributed_cache_trust")]
    pub distributed_cache_trust: String,

    /// Enable profiling of slow functions
    #[serde(default = "default_true")]
    pub profile_slow_functions: bool,

    /// Profiling threshold (e.g., "1s")
    #[serde(default = "default_profile_threshold")]
    pub profile_threshold: String,
}

impl Default for VerifySection {
    fn default() -> Self {
        Self {
            total_budget: None,
            slow_threshold: default_slow_threshold(),
            cache_dir: default_cache_dir(),
            cache_max_size: default_cache_max_size(),
            cache_ttl: default_cache_ttl(),
            distributed_cache: None,
            distributed_cache_trust: default_distributed_cache_trust(),
            profile_slow_functions: true,
            profile_threshold: default_profile_threshold(),
        }
    }
}

// Default values
fn default_slow_threshold() -> String {
    "5s".to_string()
}

fn default_cache_dir() -> String {
    ".verum/verify-cache".to_string()
}

fn default_cache_max_size() -> String {
    "500MB".to_string()
}

fn default_cache_ttl() -> String {
    "30d".to_string()
}

fn default_distributed_cache_trust() -> String {
    "signatures".to_string()
}

fn default_profile_threshold() -> String {
    "1s".to_string()
}

fn default_true() -> bool {
    true
}

impl VerificationConfig {
    /// Load from verum.toml file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read {}", path.as_ref().display()))?;

        let config: VerificationConfig = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.as_ref().display()))?;

        Ok(config)
    }

    /// Load from current directory (looks for verum.toml)
    pub fn load() -> Result<Self> {
        let path = PathBuf::from("verum.toml");
        if path.exists() {
            Self::load_from_file(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Get total budget as Duration
    pub fn total_budget_duration(&self) -> Option<Duration> {
        self.verify
            .total_budget
            .as_ref()
            .and_then(|s| parse_duration(s))
    }

    /// Get slow threshold as Duration
    pub fn slow_threshold_duration(&self) -> Duration {
        parse_duration(&self.verify.slow_threshold).unwrap_or_else(|| Duration::from_secs(5))
    }

    /// Get profile threshold as Duration
    pub fn profile_threshold_duration(&self) -> Duration {
        parse_duration(&self.verify.profile_threshold).unwrap_or_else(|| Duration::from_secs(1))
    }

    /// Get cache TTL as Duration
    pub fn cache_ttl_duration(&self) -> Duration {
        parse_duration(&self.verify.cache_ttl)
            .unwrap_or_else(|| Duration::from_secs(30 * 24 * 60 * 60)) // 30 days
    }

    /// Get cache max size in bytes
    pub fn cache_max_size_bytes(&self) -> u64 {
        parse_byte_size(&self.verify.cache_max_size).unwrap_or(500 * 1024 * 1024) // 500MB
    }

    /// Get cache directory path
    pub fn cache_dir(&self) -> PathBuf {
        PathBuf::from(&self.verify.cache_dir)
    }

    /// Get distributed cache URL
    pub fn distributed_cache_url(&self) -> Option<Text> {
        self.verify
            .distributed_cache
            .as_ref()
            .map(|s| Text::from(s.as_str()))
    }
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            verify: VerifySection::default(),
        }
    }
}

/// Parse duration string (e.g., "120s", "5m", "1h", "30d")
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Find where numbers end and units begin
    let split_pos = s
        .chars()
        .position(|c| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());

    let (num_str, unit) = s.split_at(split_pos);
    let num: f64 = num_str.parse().ok()?;

    let multiplier = match unit.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        "d" | "day" | "days" => 86400.0,
        "" => 1.0, // Default to seconds
        _ => return None,
    };

    let secs = num * multiplier;
    Some(Duration::from_secs_f64(secs))
}

/// Parse byte size string (e.g., "500MB", "1GB", "10KB")
fn parse_byte_size(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }

    // Find where numbers end and units begin
    let split_pos = s
        .chars()
        .position(|c| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());

    let (num_str, unit) = s.split_at(split_pos);
    let num: f64 = num_str.parse().ok()?;

    let multiplier: u64 = match unit.trim() {
        "B" | "BYTES" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        "TB" | "T" => 1024_u64 * 1024 * 1024 * 1024,
        "" => 1, // Default to bytes
        _ => return None,
    };

    Some((num * multiplier as f64) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("120s"), Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(parse_duration("30d"), Some(Duration::from_secs(30 * 86400)));
        assert_eq!(
            parse_duration("2.5h"),
            Some(Duration::from_secs_f64(2.5 * 3600.0))
        );
    }

    #[test]
    fn test_parse_byte_size() {
        assert_eq!(parse_byte_size("500MB"), Some(500 * 1024 * 1024));
        assert_eq!(parse_byte_size("1GB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_byte_size("10KB"), Some(10 * 1024));
        assert_eq!(parse_byte_size("100"), Some(100));
    }

    #[test]
    fn test_default_config() {
        let config = VerificationConfig::default();
        assert_eq!(config.verify.slow_threshold, "5s");
        assert_eq!(config.verify.cache_max_size, "500MB");
        assert_eq!(config.verify.cache_ttl, "30d");
        assert!(config.verify.profile_slow_functions);
    }

    #[test]
    fn test_parse_toml() {
        let toml_str = r#"
[verify]
total_budget = "120s"
slow_threshold = "5s"
cache_dir = ".verum/verify-cache"
cache_max_size = "500MB"
cache_ttl = "30d"
distributed_cache = "s3://my-bucket/verum-cache"
distributed_cache_trust = "signatures"
profile_slow_functions = true
profile_threshold = "1s"
"#;

        let config: VerificationConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.verify.total_budget, Some("120s".to_string()));
        assert_eq!(
            config.total_budget_duration(),
            Some(Duration::from_secs(120))
        );
        assert_eq!(config.cache_max_size_bytes(), 500 * 1024 * 1024);
    }
}
