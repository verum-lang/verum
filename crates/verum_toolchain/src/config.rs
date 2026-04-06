//! Global and toolchain configuration.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::error::ToolchainResult;

/// Global Verum configuration (~/.verum/config.toml).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GlobalConfig {
    /// Active toolchain name (default: "dev" in dev mode, latest stable otherwise).
    #[serde(default)]
    pub default_toolchain: Option<String>,

    /// Default compilation target (default: host target).
    #[serde(default)]
    pub default_target: Option<String>,

    /// Enable LTO by default when bitcode is available.
    #[serde(default)]
    pub enable_lto: bool,

    /// Enable verbose logging.
    #[serde(default)]
    pub verbose: bool,

    /// Custom download mirror URL.
    #[serde(default)]
    pub download_mirror: Option<String>,

    /// Proxy settings.
    #[serde(default)]
    pub proxy: Option<ProxyConfig>,

    /// Cache settings.
    #[serde(default)]
    pub cache: CacheConfig,
}

/// Proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    /// HTTP proxy URL.
    pub http: Option<String>,
    /// HTTPS proxy URL.
    pub https: Option<String>,
}

/// Cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Maximum cache size in MB (0 = unlimited).
    #[serde(default)]
    pub max_size_mb: u64,

    /// Cache TTL in days (0 = never expire).
    #[serde(default)]
    pub ttl_days: u32,

    /// Enable incremental compilation cache.
    #[serde(default = "default_true")]
    pub incremental: bool,

    /// Enable monomorphization cache.
    #[serde(default = "default_true")]
    pub monomorphization: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size_mb: 0,       // Unlimited
            ttl_days: 0,         // Never expire
            incremental: true,
            monomorphization: true,
        }
    }
}

fn default_true() -> bool {
    true
}

impl GlobalConfig {
    /// Load global config from path.
    pub fn load(path: &Path) -> ToolchainResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Save global config to path.
    pub fn save(&self, path: &Path) -> ToolchainResult<()> {
        let contents = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}

/// Per-toolchain configuration (inside toolchain directory).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolchainConfig {
    /// Custom linker flags.
    #[serde(default)]
    pub linker_flags: Vec<String>,

    /// Custom compiler flags.
    #[serde(default)]
    pub compiler_flags: Vec<String>,

    /// Additional library search paths.
    #[serde(default)]
    pub library_paths: Vec<String>,
}

impl ToolchainConfig {
    /// Load toolchain config from path.
    pub fn load(path: &Path) -> ToolchainResult<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_global_config_default() {
        let config = GlobalConfig::default();
        assert!(config.default_toolchain.is_none());
        assert!(config.cache.incremental);
    }

    #[test]
    fn test_global_config_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        let config = GlobalConfig {
            default_toolchain: Some("dev".to_string()),
            enable_lto: true,
            ..Default::default()
        };

        config.save(&path).unwrap();

        let loaded = GlobalConfig::load(&path).unwrap();
        assert_eq!(loaded.default_toolchain, Some("dev".to_string()));
        assert!(loaded.enable_lto);
    }
}
