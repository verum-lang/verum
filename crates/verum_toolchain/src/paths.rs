//! Verum directory paths.

use std::path::PathBuf;

use crate::error::{ToolchainError, ToolchainResult};

/// Standard Verum directory paths.
#[derive(Debug, Clone)]
pub struct VerumPaths {
    /// Base Verum home directory (~/.verum).
    pub home: PathBuf,

    /// Toolchains directory (~/.verum/toolchains).
    pub toolchains: PathBuf,

    /// Cache directory (~/.verum/cache).
    pub cache: PathBuf,

    /// Downloads directory (~/.verum/downloads).
    pub downloads: PathBuf,
}

impl VerumPaths {
    /// Create paths from default home directory (~/.verum).
    pub fn new() -> ToolchainResult<Self> {
        let home = Self::default_home()?;
        Ok(Self::from_home(home))
    }

    /// Create paths from custom home directory.
    pub fn from_home(home: PathBuf) -> Self {
        Self {
            toolchains: home.join("toolchains"),
            cache: home.join("cache"),
            downloads: home.join("downloads"),
            home,
        }
    }

    /// Get the default Verum home directory.
    ///
    /// Priority:
    /// 1. `VERUM_HOME` environment variable
    /// 2. `~/.verum`
    pub fn default_home() -> ToolchainResult<PathBuf> {
        // Check environment variable first
        if let Ok(home) = std::env::var("VERUM_HOME") {
            return Ok(PathBuf::from(home));
        }

        // Fall back to ~/.verum
        let home = dirs::home_dir().ok_or(ToolchainError::NoHomeDir)?;
        Ok(home.join(".verum"))
    }

    /// Get path to a specific toolchain.
    pub fn toolchain_path(&self, name: &str) -> PathBuf {
        self.toolchains.join(name)
    }

    /// Get path to toolchain's library directory for a target.
    pub fn toolchain_lib_path(&self, name: &str, target: &str) -> PathBuf {
        self.toolchains.join(name).join("lib").join(target)
    }

    /// Get path to toolchain's stdlib directory.
    pub fn toolchain_stdlib_path(&self, name: &str) -> PathBuf {
        self.toolchains.join(name).join("stdlib")
    }

    /// Get path to monomorphization cache.
    pub fn mono_cache(&self) -> PathBuf {
        self.cache.join("mono")
    }

    /// Get path to incremental compilation cache.
    pub fn incremental_cache(&self) -> PathBuf {
        self.cache.join("incremental")
    }

    /// Get path to global config file.
    pub fn config_path(&self) -> PathBuf {
        self.home.join("config.toml")
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> ToolchainResult<()> {
        std::fs::create_dir_all(&self.home)?;
        std::fs::create_dir_all(&self.toolchains)?;
        std::fs::create_dir_all(&self.cache)?;
        std::fs::create_dir_all(&self.downloads)?;
        Ok(())
    }

    /// Get the host target triple.
    pub fn host_target() -> String {
        // Use build-time target from environment (set by build.rs)
        // Fall back to compile-time constant
        std::env::var("VERUM_HOST_TARGET")
            .unwrap_or_else(|_| {
                // Compile-time target detection
                #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
                { "x86_64-apple-darwin".to_string() }
                #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
                { "aarch64-apple-darwin".to_string() }
                #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                { "x86_64-unknown-linux-gnu".to_string() }
                #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
                { "aarch64-unknown-linux-gnu".to_string() }
                #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
                { "x86_64-pc-windows-msvc".to_string() }
                #[cfg(not(any(
                    all(target_arch = "x86_64", target_os = "macos"),
                    all(target_arch = "aarch64", target_os = "macos"),
                    all(target_arch = "x86_64", target_os = "linux"),
                    all(target_arch = "aarch64", target_os = "linux"),
                    all(target_arch = "x86_64", target_os = "windows"),
                )))]
                { "unknown-unknown-unknown".to_string() }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths_structure() {
        let home = PathBuf::from("/tmp/test-verum");
        let paths = VerumPaths::from_home(home.clone());

        assert_eq!(paths.home, home);
        assert_eq!(paths.toolchains, home.join("toolchains"));
        assert_eq!(paths.cache, home.join("cache"));
    }

    #[test]
    fn test_toolchain_paths() {
        let paths = VerumPaths::from_home(PathBuf::from("/tmp/verum"));

        assert_eq!(
            paths.toolchain_path("dev"),
            PathBuf::from("/tmp/verum/toolchains/dev")
        );
        assert_eq!(
            paths.toolchain_lib_path("dev", "x86_64-apple-darwin"),
            PathBuf::from("/tmp/verum/toolchains/dev/lib/x86_64-apple-darwin")
        );
    }

    #[test]
    fn test_host_target() {
        let target = VerumPaths::host_target();
        assert!(!target.is_empty());
        assert!(target.contains('-'));
    }
}
