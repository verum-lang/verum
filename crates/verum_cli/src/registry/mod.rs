// Verum cog registry client.
// Implements comprehensive cog distribution with multi-source support:
// registry (centralized), git repositories, local paths, and IPFS (decentralized).

pub mod cache_manager;
pub mod client;
pub mod content_store;
pub mod lockfile_v3;
pub mod enterprise;
pub mod ipfs;
pub mod lockfile;
pub mod mirror;
pub mod resolver;
pub mod sat_resolver;
pub mod security;
pub mod signing;
pub mod types;

pub use cache_manager::CacheManager;
pub use client::RegistryClient;
pub use enterprise::EnterpriseClient;
pub use ipfs::IpfsClient;
pub use lockfile::{LockedCog, Lockfile};
pub use resolver::{DependencyResolver, resolve_version};
pub use sat_resolver::SatResolver;
pub use security::SecurityScanner;
pub use signing::CogSigner;
pub use types::*;

use crate::error::Result;

/// Default Verum cog registry
pub const DEFAULT_REGISTRY: &str = "https://packages.verum.lang";

/// Registry index URL
pub fn registry_index_url(base: &str) -> String {
    format!("{}/index", base)
}

/// Registry API URL
pub fn registry_api_url(base: &str) -> String {
    format!("{}/api/v1", base)
}

/// Get local cog cache directory
pub fn cache_dir() -> Result<std::path::PathBuf> {
    let cache = dirs::cache_dir()
        .ok_or_else(|| crate::error::CliError::Custom("Cannot determine cache directory".into()))?;
    Ok(cache.join("verum").join("cogs"))
}

/// Get local registry mirror directory
pub fn mirror_dir() -> Result<std::path::PathBuf> {
    let data = dirs::data_local_dir()
        .ok_or_else(|| crate::error::CliError::Custom("Cannot determine data directory".into()))?;
    Ok(data.join("verum").join("mirror"))
}
