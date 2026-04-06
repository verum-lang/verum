//! Verum Toolchain Management
//!
//! This crate manages Verum toolchains stored in `~/.verum/toolchains/`.
//!
//! A toolchain contains the standard library bytecode. Runtime support is
//! provided by stdlib via FFI and intrinsics.
//!
//! Each target has its own stdlib.vbca because:
//! - Platform-specific intrinsics generate different bytecode
//! - Conditional compilation (@cfg) produces target-specific code
//! - Cross-compilation requires downloading target-specific toolchains
//!
//! ## Directory Structure
//!
//! ```text
//! ~/.verum/
//! ├── toolchains/
//! │   ├── dev/                          # Development toolchain (local build)
//! │   │   ├── manifest.json             # Version and checksums
//! │   │   └── lib/
//! │   │       └── {target}/             # Per-platform artifacts
//! │   │           └── stdlib.vbca       # Compiled standard library
//! │   └── v0.4.0/                       # Released toolchain (downloaded)
//! │       └── ...
//! ├── cache/
//! │   ├── mono/                         # Monomorphization cache
//! │   └── incremental/                  # Incremental compilation cache
//! ├── downloads/                        # Downloaded archives
//! └── config.toml                       # Global configuration
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use verum_toolchain::{ToolchainManager, Toolchain};
//!
//! // Get the default toolchain manager
//! let manager = ToolchainManager::new()?;
//!
//! // Get the active toolchain
//! let toolchain = manager.active_toolchain()?;
//!
//! // Get stdlib for host platform
//! let stdlib = toolchain.stdlib()?;
//! println!("Stdlib: {}", stdlib.path.display());
//!
//! // Get stdlib for specific target (cross-compilation)
//! let stdlib = toolchain.stdlib_for_target("x86_64-unknown-linux-gnu")?;
//! ```

#![allow(missing_docs)]
#![deny(rust_2018_idioms)]

mod config;
mod error;
mod manifest;
mod paths;
mod runtime;
mod stdlib;
mod toolchain;

pub use config::{ToolchainConfig, GlobalConfig};
pub use error::{ToolchainError, ToolchainResult};
pub use manifest::{ToolchainManifest, LibraryChecksums, ToolchainSource, StdlibManifest};
pub use paths::VerumPaths;
pub use stdlib::StdlibArtifacts;
pub use toolchain::{Toolchain, ToolchainManager, ToolchainInfo};

// DEPRECATED: RuntimeArtifacts kept for backward compatibility
#[deprecated(note = "Runtime is now part of stdlib. RuntimeArtifacts is no longer needed.")]
#[allow(deprecated)]
pub use runtime::RuntimeArtifacts;
pub use runtime::compute_data_checksum;

/// Verum version (for cache invalidation)
pub const VERUM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Toolchain manifest format version
pub const MANIFEST_FORMAT_VERSION: u32 = 1;

/// Default toolchain name for development builds
pub const DEV_TOOLCHAIN_NAME: &str = "dev";
