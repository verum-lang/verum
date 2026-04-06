//! Toolchain manifest for version tracking and validation.
//!
//! ## Architecture (v2 - stdlib-only)
//!
//! The toolchain now contains only the standard library bytecode.
//! Runtime support (previously libverum_bootstrap.a, libverum_runtime.a)
//! has been moved into the stdlib itself via FFI and intrinsics.
//!
//! Each target has its own stdlib.vbca because:
//! - Platform-specific intrinsics generate different bytecode
//! - Conditional compilation (@cfg) produces target-specific code
//! - Cross-compilation requires downloading target-specific toolchains

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::{ToolchainError, ToolchainResult};
use crate::{MANIFEST_FORMAT_VERSION, VERUM_VERSION};

/// Toolchain manifest stored in each toolchain directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainManifest {
    /// Manifest format version (for schema migrations).
    /// v2: stdlib-only, per-target stdlib.vbca
    pub format_version: u32,

    /// Verum version that created this toolchain.
    pub verum_version: String,

    /// Toolchain name (e.g., "dev", "v0.4.0").
    pub name: String,

    /// Source of the toolchain.
    pub source: ToolchainSource,

    /// When the toolchain was created/installed.
    pub created_at: String,

    /// Git commit hash (for dev toolchain).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,

    /// Available targets with per-target stdlib.
    pub targets: HashMap<String, TargetManifest>,

    /// DEPRECATED: Global stdlib info (v1 format).
    /// In v2, stdlib is per-target in `targets[target].stdlib`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdlib: Option<StdlibManifest>,
}

/// Source of the toolchain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainSource {
    /// Built locally during development.
    LocalBuild,
    /// Downloaded from GitHub releases.
    Downloaded {
        url: String,
    },
    /// Manually installed by user.
    Manual,
}

/// Per-target information including stdlib.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetManifest {
    /// Target triple (e.g., "x86_64-apple-darwin").
    pub target: String,

    /// Stdlib for this target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdlib: Option<StdlibManifest>,

    /// When this target was built/installed.
    pub built_at: String,

    /// DEPRECATED: Legacy library checksums (v1 format).
    /// In v2, only stdlib is needed - runtime is in stdlib via intrinsics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libraries: Option<LibraryChecksums>,
}

/// SHA-256 checksums for legacy libraries.
/// DEPRECATED: Only kept for backward compatibility with v1 manifests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryChecksums {
    pub libverum_bootstrap: String,
    pub libverum_runtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libverum_bootstrap_bc: Option<String>,
}

/// Standard library manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdlibManifest {
    /// Checksum of stdlib.vbca.
    pub checksum: String,
    /// VBC version.
    pub vbc_version: u32,
    /// Number of modules in stdlib.
    pub module_count: u32,
    /// When stdlib was compiled.
    pub built_at: String,
}

impl ToolchainManifest {
    /// Create a new manifest for a local dev build.
    pub fn new_dev(git_commit: Option<String>) -> Self {
        Self {
            format_version: MANIFEST_FORMAT_VERSION,
            verum_version: VERUM_VERSION.to_string(),
            name: "dev".to_string(),
            source: ToolchainSource::LocalBuild,
            created_at: current_timestamp(),
            git_commit,
            targets: HashMap::new(),
            stdlib: None,
        }
    }

    /// Create a new manifest for a downloaded toolchain.
    pub fn new_downloaded(name: String, url: String) -> Self {
        Self {
            format_version: MANIFEST_FORMAT_VERSION,
            verum_version: VERUM_VERSION.to_string(),
            name,
            source: ToolchainSource::Downloaded { url },
            created_at: current_timestamp(),
            git_commit: None,
            targets: HashMap::new(),
            stdlib: None,
        }
    }

    /// Load manifest from file.
    pub fn load(path: &Path) -> ToolchainResult<Self> {
        let contents = std::fs::read_to_string(path)?;
        let manifest: Self = serde_json::from_str(&contents)?;
        Ok(manifest)
    }

    /// Save manifest to file.
    pub fn save(&self, path: &Path) -> ToolchainResult<()> {
        let contents = serde_json::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Validate manifest against current version.
    pub fn validate(&self) -> ToolchainResult<()> {
        // Check format version
        if self.format_version != MANIFEST_FORMAT_VERSION {
            return Err(ToolchainError::ManifestInvalid {
                toolchain: self.name.clone(),
                reason: format!(
                    "Format version mismatch: expected {}, found {}",
                    MANIFEST_FORMAT_VERSION, self.format_version
                ),
            });
        }

        // Check Verum version compatibility
        // For dev builds, we always regenerate
        if self.source != ToolchainSource::LocalBuild && self.verum_version != VERUM_VERSION {
            return Err(ToolchainError::VersionMismatch {
                required: self.verum_version.clone(),
                current: VERUM_VERSION.to_string(),
            });
        }

        Ok(())
    }

    /// Add a target with stdlib.
    pub fn add_target_with_core(
        &mut self,
        target: String,
        checksum: String,
        vbc_version: u32,
        module_count: u32,
    ) {
        let stdlib = StdlibManifest {
            checksum,
            vbc_version,
            module_count,
            built_at: current_timestamp(),
        };
        self.targets.insert(target.clone(), TargetManifest {
            target,
            stdlib: Some(stdlib),
            built_at: current_timestamp(),
            libraries: None,
        });
    }

    /// DEPRECATED: Add or update target with legacy library checksums.
    /// Use `add_target_with_core` for new code.
    pub fn add_target(&mut self, target: String, checksums: LibraryChecksums) {
        self.targets.insert(target.clone(), TargetManifest {
            target,
            stdlib: None,
            built_at: current_timestamp(),
            libraries: Some(checksums),
        });
    }

    /// DEPRECATED: Set global stdlib information (v1 format).
    /// Use `add_target_with_core` for per-target stdlib.
    pub fn set_stdlib(&mut self, checksum: String, vbc_version: u32, module_count: u32) {
        self.stdlib = Some(StdlibManifest {
            checksum,
            vbc_version,
            module_count,
            built_at: current_timestamp(),
        });
    }

    /// Set stdlib for a specific target.
    pub fn set_target_stdlib(
        &mut self,
        target: &str,
        checksum: String,
        vbc_version: u32,
        module_count: u32,
    ) {
        let stdlib = StdlibManifest {
            checksum,
            vbc_version,
            module_count,
            built_at: current_timestamp(),
        };

        if let Some(target_manifest) = self.targets.get_mut(target) {
            target_manifest.stdlib = Some(stdlib);
        } else {
            // Create new target entry
            self.targets.insert(target.to_string(), TargetManifest {
                target: target.to_string(),
                stdlib: Some(stdlib),
                built_at: current_timestamp(),
                libraries: None,
            });
        }
    }

    /// Check if target is available.
    pub fn has_target(&self, target: &str) -> bool {
        self.targets.contains_key(target)
    }

    /// Check if stdlib is available for a specific target.
    pub fn has_target_stdlib(&self, target: &str) -> bool {
        self.targets
            .get(target)
            .and_then(|t| t.stdlib.as_ref())
            .is_some()
    }

    /// Check if any stdlib is available (global or per-target).
    pub fn has_stdlib(&self) -> bool {
        // Check global stdlib (v1 format)
        if self.stdlib.is_some() {
            return true;
        }
        // Check per-target stdlib (v2 format)
        self.targets.values().any(|t| t.stdlib.is_some())
    }

    /// Get stdlib manifest for a specific target.
    /// Falls back to global stdlib if per-target not available.
    pub fn get_stdlib(&self, target: &str) -> Option<&StdlibManifest> {
        // Try per-target first (v2)
        if let Some(target_manifest) = self.targets.get(target)
            && let Some(ref stdlib) = target_manifest.stdlib
        {
            return Some(stdlib);
        }
        // Fall back to global (v1)
        self.stdlib.as_ref()
    }
}

/// Get current timestamp as ISO 8601 string.
fn current_timestamp() -> String {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Simple timestamp without chrono
    format!("{}", now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_manifest_v2_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("manifest.json");

        let mut manifest = ToolchainManifest::new_dev(Some("abc123".to_string()));
        manifest.add_target_with_core(
            "x86_64-apple-darwin".to_string(),
            "stdlib_hash".to_string(),
            1,
            50,
        );

        manifest.save(&path).unwrap();

        let loaded = ToolchainManifest::load(&path).unwrap();
        assert_eq!(loaded.name, "dev");
        assert_eq!(loaded.git_commit, Some("abc123".to_string()));
        assert!(loaded.has_target("x86_64-apple-darwin"));
        assert!(loaded.has_target_stdlib("x86_64-apple-darwin"));
        assert!(loaded.has_stdlib());

        // Check per-target stdlib
        let stdlib = loaded.get_stdlib("x86_64-apple-darwin").unwrap();
        assert_eq!(stdlib.checksum, "stdlib_hash");
        assert_eq!(stdlib.module_count, 50);
    }

    #[test]
    fn test_manifest_v1_backward_compat() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("manifest.json");

        // Simulate v1 format with global stdlib and library checksums
        let mut manifest = ToolchainManifest::new_dev(Some("abc123".to_string()));
        manifest.add_target(
            "x86_64-apple-darwin".to_string(),
            LibraryChecksums {
                libverum_bootstrap: "hash1".to_string(),
                libverum_runtime: "hash2".to_string(),
                libverum_bootstrap_bc: Some("hash3".to_string()),
            },
        );
        manifest.set_stdlib("stdlib_hash".to_string(), 1, 50);

        manifest.save(&path).unwrap();

        let loaded = ToolchainManifest::load(&path).unwrap();
        assert!(loaded.has_target("x86_64-apple-darwin"));
        assert!(loaded.has_stdlib());

        // Should fall back to global stdlib for any target
        let stdlib = loaded.get_stdlib("x86_64-apple-darwin").unwrap();
        assert_eq!(stdlib.checksum, "stdlib_hash");
    }

    #[test]
    fn test_manifest_validation() {
        let manifest = ToolchainManifest::new_dev(None);
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_set_target_stdlib() {
        let mut manifest = ToolchainManifest::new_dev(None);

        // Set stdlib for a target that doesn't exist yet
        manifest.set_target_stdlib(
            "aarch64-apple-darwin",
            "arm_stdlib_hash".to_string(),
            1,
            100,
        );

        assert!(manifest.has_target("aarch64-apple-darwin"));
        assert!(manifest.has_target_stdlib("aarch64-apple-darwin"));

        let stdlib = manifest.get_stdlib("aarch64-apple-darwin").unwrap();
        assert_eq!(stdlib.checksum, "arm_stdlib_hash");
    }
}
