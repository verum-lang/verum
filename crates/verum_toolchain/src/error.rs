//! Toolchain error types.

use std::path::PathBuf;
use thiserror::Error;

/// Result type for toolchain operations.
pub type ToolchainResult<T> = Result<T, ToolchainError>;

/// Toolchain errors.
#[derive(Debug, Error)]
pub enum ToolchainError {
    /// No toolchain found.
    #[error("No toolchain found. Run 'verum toolchain install' or build in development mode")]
    NoToolchainFound,

    /// Toolchain not found by name.
    #[error("Toolchain '{name}' not found. Available: {available:?}")]
    ToolchainNotFound {
        name: String,
        available: Vec<String>,
    },

    /// Missing runtime libraries for target.
    #[error("Runtime libraries not found for target '{target}' in toolchain '{toolchain}'")]
    RuntimeNotFound {
        toolchain: String,
        target: String,
    },

    /// Missing stdlib.
    #[error("Standard library not found in toolchain '{toolchain}'")]
    StdlibNotFound {
        toolchain: String,
    },

    /// Missing library file.
    #[error("Library '{library}' not found at {path}")]
    LibraryNotFound {
        library: String,
        path: PathBuf,
    },

    /// Manifest validation failed.
    #[error("Manifest validation failed for toolchain '{toolchain}': {reason}")]
    ManifestInvalid {
        toolchain: String,
        reason: String,
    },

    /// Version mismatch.
    #[error("Version mismatch: toolchain requires {required}, but current is {current}")]
    VersionMismatch {
        required: String,
        current: String,
    },

    /// Checksum mismatch.
    #[error("Checksum mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },

    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// TOML serialization error.
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    /// Home directory not found.
    #[error("Could not determine home directory")]
    NoHomeDir,

    /// Cross-compilation not available.
    #[error(
        "Cross-compilation runtime for '{target}' not available.\n\
         Download with: verum toolchain add-target {target}\n\
         Or install from: {download_url}"
    )]
    CrossCompilationNotAvailable {
        target: String,
        download_url: String,
    },

    /// Build artifacts not found (dev mode).
    #[error(
        "Development toolchain not found at {path}.\n\
         In development mode, run 'cargo build' first to compile runtime libraries."
    )]
    DevToolchainNotBuilt {
        path: PathBuf,
    },
}
