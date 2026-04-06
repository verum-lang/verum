//! Runtime library artifacts.
//!
//! # DEPRECATED
//!
//! This module is deprecated. Runtime functionality has been moved into
//! the standard library via FFI and intrinsics.
//!
//! The only artifacts now are:
//! - `stdlib.vbca` - Compiled standard library bytecode (per-target)
//!
//! This module is kept for:
//! - Backward compatibility with legacy toolchains
//! - Checksum utilities (`compute_data_checksum`, `compute_file_checksum`)

// Allow deprecated usage within this module since we're implementing the deprecated API
#![allow(deprecated)]

use std::path::{Path, PathBuf};
use sha2::{Sha256, Digest};
use std::io::Read;

use crate::error::{ToolchainError, ToolchainResult};
use crate::manifest::LibraryChecksums;

/// DEPRECATED: Runtime library artifacts for a specific target.
///
/// This is no longer used. Runtime is part of stdlib.
/// Kept for backward compatibility with legacy toolchains.
#[deprecated(note = "Runtime is now part of stdlib. Use StdlibArtifacts instead.")]
#[derive(Debug, Clone)]
pub struct RuntimeArtifacts {
    /// Base directory containing the libraries.
    pub path: PathBuf,

    /// Path to libverum_bootstrap.a (core types, intrinsics, refs).
    pub libverum_bootstrap: PathBuf,

    /// Path to libverum_runtime.a (async, context system).
    pub libverum_runtime: PathBuf,

    /// Path to libverum_bootstrap.bc (LLVM bitcode for LTO, optional).
    pub libverum_bootstrap_bc: Option<PathBuf>,

    /// Target triple for these artifacts.
    pub target: String,
}

impl RuntimeArtifacts {
    /// Create artifacts from a directory path.
    pub fn from_dir(path: PathBuf, target: String) -> Self {
        let libverum_bootstrap = path.join("libverum_bootstrap.a");
        let libverum_runtime = path.join("libverum_runtime.a");
        let libverum_bootstrap_bc_path = path.join("libverum_bootstrap.bc");
        let libverum_bootstrap_bc = if libverum_bootstrap_bc_path.exists() {
            Some(libverum_bootstrap_bc_path)
        } else {
            None
        };

        Self {
            path,
            libverum_bootstrap,
            libverum_runtime,
            libverum_bootstrap_bc,
            target,
        }
    }

    /// Validate that all required libraries exist.
    pub fn validate(&self) -> ToolchainResult<()> {
        self.check_exists(&self.libverum_bootstrap, "libverum_bootstrap.a")?;
        self.check_exists(&self.libverum_runtime, "libverum_runtime.a")?;
        Ok(())
    }

    /// Check if a library exists.
    fn check_exists(&self, path: &Path, name: &str) -> ToolchainResult<()> {
        if !path.exists() {
            return Err(ToolchainError::LibraryNotFound {
                library: name.to_string(),
                path: path.to_path_buf(),
            });
        }
        Ok(())
    }

    /// Verify checksums match the manifest.
    pub fn verify_checksums(&self, checksums: &LibraryChecksums) -> ToolchainResult<()> {
        self.verify_file(&self.libverum_bootstrap, &checksums.libverum_bootstrap, "libverum_bootstrap.a")?;
        self.verify_file(&self.libverum_runtime, &checksums.libverum_runtime, "libverum_runtime.a")?;

        if let (Some(path), Some(expected)) = (&self.libverum_bootstrap_bc, &checksums.libverum_bootstrap_bc) {
            self.verify_file(path, expected, "libverum_bootstrap.bc")?;
        }

        Ok(())
    }

    /// Verify a single file's checksum.
    fn verify_file(&self, path: &PathBuf, expected: &str, name: &str) -> ToolchainResult<()> {
        let actual = compute_file_checksum(path)?;
        if actual != expected {
            return Err(ToolchainError::ChecksumMismatch {
                file: name.to_string(),
                expected: expected.to_string(),
                actual,
            });
        }
        Ok(())
    }

    /// Compute checksums for all libraries.
    pub fn compute_checksums(&self) -> ToolchainResult<LibraryChecksums> {
        Ok(LibraryChecksums {
            libverum_bootstrap: compute_file_checksum(&self.libverum_bootstrap)?,
            libverum_runtime: compute_file_checksum(&self.libverum_runtime)?,
            libverum_bootstrap_bc: self.libverum_bootstrap_bc.as_ref()
                .map(compute_file_checksum)
                .transpose()?,
        })
    }

    /// Get library search path for linker (-L flag).
    pub fn library_path(&self) -> &PathBuf {
        &self.path
    }

    /// Check if bitcode is available for LTO.
    pub fn has_bitcode(&self) -> bool {
        self.libverum_bootstrap_bc.as_ref().map(|p| p.exists()).unwrap_or(false)
    }

    /// Get total size of all libraries in bytes.
    pub fn total_size(&self) -> u64 {
        let mut size = 0;
        if let Ok(meta) = std::fs::metadata(&self.libverum_bootstrap) {
            size += meta.len();
        }
        if let Ok(meta) = std::fs::metadata(&self.libverum_runtime) {
            size += meta.len();
        }
        if let Some(bc) = &self.libverum_bootstrap_bc
            && let Ok(meta) = std::fs::metadata(bc) {
                size += meta.len();
            }
        size
    }
}

/// Compute SHA-256 checksum of a file.
pub fn compute_file_checksum(path: &PathBuf) -> ToolchainResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Compute SHA-256 checksum of in-memory data.
pub fn compute_data_checksum(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::io::Write;

    #[test]
    fn test_runtime_artifacts_structure() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        // Create dummy files
        std::fs::write(path.join("libverum_bootstrap.a"), b"bootstrap").unwrap();
        std::fs::write(path.join("libverum_runtime.a"), b"runtime").unwrap();

        let artifacts = RuntimeArtifacts::from_dir(path.clone(), "test-target".to_string());

        assert!(artifacts.validate().is_ok());
        assert!(!artifacts.has_bitcode());
    }

    #[test]
    fn test_checksum_computation() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");

        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"test content").unwrap();

        let checksum = compute_file_checksum(&path).unwrap();
        assert_eq!(checksum.len(), 64); // SHA-256 = 64 hex chars

        // Verify consistency
        let checksum2 = compute_file_checksum(&path).unwrap();
        assert_eq!(checksum, checksum2);
    }

    #[test]
    fn test_data_checksum() {
        let data = b"test data";
        let checksum = compute_data_checksum(data);
        assert_eq!(checksum.len(), 64);

        // Same data = same checksum
        let checksum2 = compute_data_checksum(data);
        assert_eq!(checksum, checksum2);
    }
}
