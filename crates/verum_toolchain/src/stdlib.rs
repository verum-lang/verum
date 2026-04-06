//! Standard library artifacts.

use std::path::PathBuf;

use crate::error::{ToolchainError, ToolchainResult};
use crate::runtime::compute_file_checksum;

/// Standard library artifacts.
#[derive(Debug, Clone)]
pub struct StdlibArtifacts {
    /// Base directory containing stdlib.
    pub path: PathBuf,

    /// Path to stdlib.vbca (compiled VBC archive).
    pub stdlib_vbca: PathBuf,
}

impl StdlibArtifacts {
    /// Create artifacts from a directory path.
    pub fn from_dir(path: PathBuf) -> Self {
        let stdlib_vbca = path.join("core.vbca");
        Self {
            path,
            stdlib_vbca,
        }
    }

    /// Validate that stdlib exists.
    pub fn validate(&self) -> ToolchainResult<()> {
        if !self.stdlib_vbca.exists() {
            return Err(ToolchainError::LibraryNotFound {
                library: "core.vbca".to_string(),
                path: self.stdlib_vbca.clone(),
            });
        }
        Ok(())
    }

    /// Check if stdlib exists.
    pub fn exists(&self) -> bool {
        self.stdlib_vbca.exists()
    }

    /// Compute checksum of stdlib.vbca.
    pub fn compute_checksum(&self) -> ToolchainResult<String> {
        compute_file_checksum(&self.stdlib_vbca)
    }

    /// Verify checksum matches expected.
    pub fn verify_checksum(&self, expected: &str) -> ToolchainResult<()> {
        let actual = self.compute_checksum()?;
        if actual != expected {
            return Err(ToolchainError::ChecksumMismatch {
                file: "core.vbca".to_string(),
                expected: expected.to_string(),
                actual,
            });
        }
        Ok(())
    }

    /// Get size of stdlib in bytes.
    pub fn size(&self) -> u64 {
        std::fs::metadata(&self.stdlib_vbca)
            .map(|m| m.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_stdlib_artifacts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        // Create dummy stdlib
        std::fs::write(path.join("core.vbca"), b"vbca").unwrap();

        let artifacts = StdlibArtifacts::from_dir(path);
        assert!(artifacts.exists());
        assert!(artifacts.validate().is_ok());
        assert!(artifacts.size() > 0);
    }

    #[test]
    fn test_stdlib_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        let artifacts = StdlibArtifacts::from_dir(path);
        assert!(!artifacts.exists());
        assert!(artifacts.validate().is_err());
    }
}
