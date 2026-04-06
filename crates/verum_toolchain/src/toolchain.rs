//! Toolchain management.

use std::path::PathBuf;
use parking_lot::RwLock;
use tracing::debug;

use crate::config::GlobalConfig;
use crate::error::{ToolchainError, ToolchainResult};
use crate::manifest::ToolchainManifest;
use crate::paths::VerumPaths;
#[allow(deprecated)]
use crate::runtime::RuntimeArtifacts;
use crate::stdlib::StdlibArtifacts;
use crate::DEV_TOOLCHAIN_NAME;

/// Information about a discovered toolchain.
#[derive(Debug, Clone)]
pub struct ToolchainInfo {
    /// Toolchain name.
    pub name: String,
    /// Path to toolchain directory.
    pub path: PathBuf,
    /// Whether the manifest is valid.
    pub valid: bool,
    /// Toolchain source type.
    pub source: String,
    /// Available targets.
    pub targets: Vec<String>,
    /// Whether stdlib is available.
    pub has_stdlib: bool,
}

/// A loaded toolchain with access to artifacts.
#[derive(Debug)]
pub struct Toolchain {
    /// Toolchain name.
    pub name: String,
    /// Path to toolchain directory.
    pub path: PathBuf,
    /// Toolchain manifest.
    pub manifest: ToolchainManifest,
}

#[allow(deprecated)] // RuntimeArtifacts is deprecated but kept for backward compatibility
impl Toolchain {
    /// Load a toolchain from its directory.
    pub fn load(path: PathBuf, name: String) -> ToolchainResult<Self> {
        let manifest_path = path.join("manifest.json");

        if !manifest_path.exists() {
            return Err(ToolchainError::DevToolchainNotBuilt { path });
        }

        let manifest = ToolchainManifest::load(&manifest_path)?;
        manifest.validate()?;

        Ok(Self { name, path, manifest })
    }

    /// DEPRECATED: Get runtime artifacts for a target.
    ///
    /// Runtime is now part of stdlib via intrinsics.
    /// This method is kept for backward compatibility with legacy manifests.
    ///
    /// # Arguments
    /// * `target` - Target triple (None = host target)
    #[deprecated(note = "Runtime is now part of stdlib. Use stdlib_for_target() instead.")]
    pub fn runtime_for_target(&self, target: Option<&str>) -> ToolchainResult<RuntimeArtifacts> {
        let host = VerumPaths::host_target();
        let target = target.unwrap_or(&host);

        // Check if target is available
        if !self.manifest.has_target(target) {
            return Err(ToolchainError::RuntimeNotFound {
                toolchain: self.name.clone(),
                target: target.to_string(),
            });
        }

        let lib_path = self.path.join("lib").join(target);
        let artifacts = RuntimeArtifacts::from_dir(lib_path, target.to_string());

        // Only validate if legacy libraries might exist
        if let Some(target_manifest) = self.manifest.targets.get(target)
            && let Some(ref libraries) = target_manifest.libraries
        {
            artifacts.validate()?;
            artifacts.verify_checksums(libraries)?;
        }

        Ok(artifacts)
    }

    /// Get stdlib artifacts for the host target.
    pub fn stdlib(&self) -> ToolchainResult<StdlibArtifacts> {
        let host = VerumPaths::host_target();
        self.stdlib_for_target(&host)
    }

    /// Get stdlib artifacts for a specific target.
    ///
    /// Stdlib is per-target (stored in lib/<target>/stdlib.vbca).
    /// For legacy manifests, falls back to the global stdlib directory.
    pub fn stdlib_for_target(&self, target: &str) -> ToolchainResult<StdlibArtifacts> {
        // Check if we have stdlib for this target
        let stdlib_manifest = self.manifest.get_stdlib(target)
            .ok_or_else(|| ToolchainError::StdlibNotFound {
                toolchain: self.name.clone(),
            })?;

        // v2: per-target stdlib in lib/<target>/
        let stdlib_path = self.path.join("lib").join(target);
        let artifacts = StdlibArtifacts::from_dir(stdlib_path.clone());

        if artifacts.exists() {
            artifacts.verify_checksum(&stdlib_manifest.checksum)?;
            return Ok(artifacts);
        }

        // v1 fallback: global stdlib directory
        let global_stdlib_path = self.path.join("stdlib");
        let global_artifacts = StdlibArtifacts::from_dir(global_stdlib_path);

        if global_artifacts.exists() {
            global_artifacts.verify_checksum(&stdlib_manifest.checksum)?;
            return Ok(global_artifacts);
        }

        Err(ToolchainError::StdlibNotFound {
            toolchain: self.name.clone(),
        })
    }

    /// Check if a target is available.
    pub fn has_target(&self, target: &str) -> bool {
        self.manifest.has_target(target)
    }

    /// Check if stdlib is available for a specific target.
    pub fn has_stdlib_for_target(&self, target: &str) -> bool {
        self.manifest.has_target_stdlib(target)
    }

    /// Check if stdlib is available (for any target).
    pub fn has_stdlib(&self) -> bool {
        self.manifest.has_stdlib()
    }

    /// List available targets.
    pub fn available_targets(&self) -> Vec<String> {
        self.manifest.targets.keys().cloned().collect()
    }

    /// List targets that have stdlib available.
    pub fn targets_with_core(&self) -> Vec<String> {
        self.manifest.targets.iter()
            .filter(|(_, t)| t.stdlib.is_some())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Get toolchain info.
    pub fn info(&self) -> ToolchainInfo {
        ToolchainInfo {
            name: self.name.clone(),
            path: self.path.clone(),
            valid: true,
            source: match &self.manifest.source {
                crate::manifest::ToolchainSource::LocalBuild => "local".to_string(),
                crate::manifest::ToolchainSource::Downloaded { url: _ } => "downloaded".to_string(),
                crate::manifest::ToolchainSource::Manual => "manual".to_string(),
            },
            targets: self.available_targets(),
            has_stdlib: self.has_stdlib(),
        }
    }
}

/// Toolchain manager for discovering and loading toolchains.
pub struct ToolchainManager {
    /// Standard Verum paths.
    paths: VerumPaths,
    /// Global configuration.
    config: GlobalConfig,
    /// Cached active toolchain.
    active: RwLock<Option<Toolchain>>,
}

impl ToolchainManager {
    /// Create a new toolchain manager with default paths.
    pub fn new() -> ToolchainResult<Self> {
        let paths = VerumPaths::new()?;
        let config = GlobalConfig::load(&paths.config_path()).unwrap_or_default();

        Ok(Self {
            paths,
            config,
            active: RwLock::new(None),
        })
    }

    /// Create a toolchain manager with custom paths.
    pub fn with_paths(paths: VerumPaths) -> ToolchainResult<Self> {
        let config = GlobalConfig::load(&paths.config_path()).unwrap_or_default();

        Ok(Self {
            paths,
            config,
            active: RwLock::new(None),
        })
    }

    /// Get the Verum paths.
    pub fn paths(&self) -> &VerumPaths {
        &self.paths
    }

    /// Get the global config.
    pub fn config(&self) -> &GlobalConfig {
        &self.config
    }

    /// List all available toolchains.
    pub fn list_toolchains(&self) -> ToolchainResult<Vec<ToolchainInfo>> {
        let mut toolchains = Vec::new();

        if !self.paths.toolchains.exists() {
            return Ok(toolchains);
        }

        for entry in std::fs::read_dir(&self.paths.toolchains)? {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            let name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let manifest_path = path.join("manifest.json");
            let info = if manifest_path.exists() {
                match ToolchainManifest::load(&manifest_path) {
                    Ok(manifest) => {
                        let valid = manifest.validate().is_ok();
                        ToolchainInfo {
                            name,
                            path,
                            valid,
                            source: match &manifest.source {
                                crate::manifest::ToolchainSource::LocalBuild => "local".to_string(),
                                crate::manifest::ToolchainSource::Downloaded { url: _ } => "downloaded".to_string(),
                                crate::manifest::ToolchainSource::Manual => "manual".to_string(),
                            },
                            targets: manifest.targets.keys().cloned().collect(),
                            has_stdlib: manifest.has_stdlib(),
                        }
                    }
                    Err(_) => ToolchainInfo {
                        name,
                        path,
                        valid: false,
                        source: "unknown".to_string(),
                        targets: vec![],
                        has_stdlib: false,
                    },
                }
            } else {
                ToolchainInfo {
                    name,
                    path,
                    valid: false,
                    source: "unknown".to_string(),
                    targets: vec![],
                    has_stdlib: false,
                }
            };

            toolchains.push(info);
        }

        Ok(toolchains)
    }

    /// Get the active toolchain.
    ///
    /// Priority:
    /// 1. VERUM_TOOLCHAIN environment variable
    /// 2. Global config default_toolchain
    /// 3. "dev" toolchain if it exists
    /// 4. Latest stable version
    pub fn active_toolchain(&self) -> ToolchainResult<Toolchain> {
        // Check cache first
        {
            let cached = self.active.read();
            if let Some(ref toolchain) = *cached {
                return Ok(Toolchain {
                    name: toolchain.name.clone(),
                    path: toolchain.path.clone(),
                    manifest: toolchain.manifest.clone(),
                });
            }
        }

        // Determine which toolchain to use
        let name = self.resolve_toolchain_name()?;
        let toolchain = self.load_toolchain(&name)?;

        // Cache it
        {
            let mut cached = self.active.write();
            *cached = Some(Toolchain {
                name: toolchain.name.clone(),
                path: toolchain.path.clone(),
                manifest: toolchain.manifest.clone(),
            });
        }

        Ok(toolchain)
    }

    /// Resolve which toolchain to use.
    fn resolve_toolchain_name(&self) -> ToolchainResult<String> {
        // 1. Environment variable
        if let Ok(name) = std::env::var("VERUM_TOOLCHAIN") {
            debug!("Using toolchain from VERUM_TOOLCHAIN: {}", name);
            return Ok(name);
        }

        // 2. Global config
        if let Some(ref name) = self.config.default_toolchain {
            debug!("Using toolchain from config: {}", name);
            return Ok(name.clone());
        }

        // 3. Check if dev toolchain exists
        let dev_path = self.paths.toolchain_path(DEV_TOOLCHAIN_NAME);
        if dev_path.exists() {
            debug!("Using dev toolchain");
            return Ok(DEV_TOOLCHAIN_NAME.to_string());
        }

        // 4. Find latest stable
        let toolchains = self.list_toolchains()?;
        let mut stable: Vec<_> = toolchains.iter()
            .filter(|t| t.valid && t.name.starts_with('v'))
            .collect();

        stable.sort_by(|a, b| b.name.cmp(&a.name)); // Descending

        if let Some(latest) = stable.first() {
            debug!("Using latest stable toolchain: {}", latest.name);
            return Ok(latest.name.clone());
        }

        Err(ToolchainError::NoToolchainFound)
    }

    /// Load a specific toolchain by name.
    pub fn load_toolchain(&self, name: &str) -> ToolchainResult<Toolchain> {
        let path = self.paths.toolchain_path(name);

        if !path.exists() {
            let available = self.list_toolchains()?
                .iter()
                .map(|t| t.name.clone())
                .collect();

            return Err(ToolchainError::ToolchainNotFound {
                name: name.to_string(),
                available,
            });
        }

        Toolchain::load(path, name.to_string())
    }

    /// DEPRECATED: Get runtime artifacts for current target.
    #[deprecated(note = "Runtime is now part of stdlib. Use stdlib() instead.")]
    #[allow(deprecated)]
    pub fn runtime(&self) -> ToolchainResult<RuntimeArtifacts> {
        let toolchain = self.active_toolchain()?;
        toolchain.runtime_for_target(None)
    }

    /// DEPRECATED: Get runtime artifacts for a specific target.
    #[deprecated(note = "Runtime is now part of stdlib. Use stdlib_for_target() instead.")]
    #[allow(deprecated)]
    pub fn runtime_for_target(&self, target: &str) -> ToolchainResult<RuntimeArtifacts> {
        let toolchain = self.active_toolchain()?;
        toolchain.runtime_for_target(Some(target))
    }

    /// Get stdlib artifacts for host target.
    pub fn stdlib(&self) -> ToolchainResult<StdlibArtifacts> {
        let toolchain = self.active_toolchain()?;
        toolchain.stdlib()
    }

    /// Get stdlib artifacts for a specific target.
    pub fn stdlib_for_target(&self, target: &str) -> ToolchainResult<StdlibArtifacts> {
        let toolchain = self.active_toolchain()?;
        toolchain.stdlib_for_target(target)
    }

    /// DEPRECATED: Check if runtime is available for host target.
    #[deprecated(note = "Use has_stdlib() instead.")]
    pub fn has_runtime(&self) -> bool {
        self.stdlib().is_ok()
    }

    /// Check if stdlib is available for host target.
    pub fn has_stdlib(&self) -> bool {
        self.stdlib().is_ok()
    }

    /// Check if stdlib is available for a specific target.
    pub fn has_stdlib_for_target(&self, target: &str) -> bool {
        self.stdlib_for_target(target).is_ok()
    }

    /// Clear the cached active toolchain.
    pub fn clear_cache(&self) {
        let mut cached = self.active.write();
        *cached = None;
    }

    /// Ensure directories exist.
    pub fn ensure_dirs(&self) -> ToolchainResult<()> {
        self.paths.ensure_dirs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_toolchain_manager_no_toolchain() {
        let dir = TempDir::new().unwrap();
        let paths = VerumPaths::from_home(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();

        let manager = ToolchainManager::with_paths(paths).unwrap();
        let result = manager.active_toolchain();

        assert!(matches!(result, Err(ToolchainError::NoToolchainFound)));
    }

    #[test]
    fn test_list_toolchains_empty() {
        let dir = TempDir::new().unwrap();
        let paths = VerumPaths::from_home(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();

        let manager = ToolchainManager::with_paths(paths).unwrap();
        let toolchains = manager.list_toolchains().unwrap();

        assert!(toolchains.is_empty());
    }
}
