//! Cross-cog module resolution.
//!
//! Maps external cog names to their filesystem roots, enabling `mount http.client.Response`
//! to resolve to the installed `http` cog's `client.vr` file.
//!
//! # Architecture
//!
//! ```text
//! mount http.client.Response;
//!   │
//!   ├── "http" matches a dependency name?
//!   │     Yes → CogResolver provides root path for "http" cog
//!   │     No  → Local module resolution (existing behavior)
//!   │
//!   └── ModuleLoader searches http_root/client.vr or http_root/client/mod.vr
//! ```
//!
//! # Resolution Strategy
//!
//! The first segment of a mount path is checked against registered cog names.
//! If it matches, the remaining segments are resolved relative to that cog's root.
//! This requires no grammar changes — existing `mount` syntax works as-is.

use std::path::PathBuf;
use verum_common::{Map, Text};

/// Location of an installed external cog.
#[derive(Debug, Clone)]
pub struct CogLocation {
    /// Cog name (e.g., "http")
    pub name: Text,
    /// Resolved version (e.g., "1.2.3")
    pub version: Text,
    /// Filesystem root path containing the cog's source files.
    /// For registry cogs: ~/.verum/cache/http-1.2.3/src/
    /// For path cogs: the relative/absolute path from Verum.toml
    /// For git cogs: ~/.verum/git/http-<rev>/src/
    pub root_path: PathBuf,
}

/// Resolves external cog names to their filesystem locations.
///
/// Built from Verum.lock (resolved dependencies) or Verum.toml (path dependencies).
/// Used by ModuleLoader to dispatch cross-cog mount statements.
#[derive(Debug, Clone)]
pub struct CogResolver {
    /// Cog name → location mapping
    cogs: Map<Text, CogLocation>,
}

impl CogResolver {
    /// Create an empty resolver (no external cogs).
    pub fn new() -> Self {
        Self {
            cogs: Map::new(),
        }
    }

    /// Register an external cog with its filesystem root.
    pub fn register_cog(&mut self, name: impl Into<Text>, version: impl Into<Text>, root_path: PathBuf) {
        let name = name.into();
        self.cogs.insert(name.clone(), CogLocation {
            name,
            version: version.into(),
            root_path,
        });
    }

    /// Check if a name refers to a registered external cog.
    pub fn is_external_cog(&self, name: &str) -> bool {
        self.cogs.contains_key(&Text::from(name))
    }

    /// Get the filesystem root for an external cog.
    pub fn get_cog_root(&self, name: &str) -> Option<&PathBuf> {
        self.cogs.get(&Text::from(name)).map(|loc| &loc.root_path)
    }

    /// Get the full location info for an external cog.
    pub fn get_cog_location(&self, name: &str) -> Option<&CogLocation> {
        self.cogs.get(&Text::from(name))
    }

    /// List all registered external cogs.
    pub fn cog_names(&self) -> Vec<&Text> {
        self.cogs.keys().collect()
    }

    /// Number of registered external cogs.
    pub fn cog_count(&self) -> usize {
        self.cogs.len()
    }

    /// Build a CogResolver from a list of locked cog entries.
    ///
    /// For each locked cog:
    /// - Registry source: maps to ~/.verum/cache/{name}-{version}/src/
    /// - Path source: uses the path directly
    /// - Git source: maps to ~/.verum/git/{name}-{rev}/src/
    /// - IPFS source: maps to ~/.verum/ipfs/{hash}/src/
    pub fn from_locked_cogs(locked_cogs: &[(Text, Text, CogSourceKind)]) -> Self {
        let mut resolver = Self::new();
        let cache_dir = Self::default_cache_dir();

        for (name, version, source) in locked_cogs {
            let root_path = match source {
                CogSourceKind::Registry => {
                    cache_dir.join(format!("{}-{}", name, version)).join("src")
                }
                CogSourceKind::Path(path) => {
                    path.clone()
                }
                CogSourceKind::Git { rev } => {
                    let short_rev = if rev.len() > 8 { &rev[..8] } else { rev.as_str() };
                    cache_dir.join("git").join(format!("{}-{}", name, short_rev)).join("src")
                }
                CogSourceKind::Ipfs { hash } => {
                    cache_dir.join("ipfs").join(hash.as_str()).join("src")
                }
            };
            resolver.register_cog(name.clone(), version.clone(), root_path);
        }

        resolver
    }

    /// Default cache directory (~/.verum/cache/).
    fn default_cache_dir() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".verum").join("cache")
    }
}

impl Default for CogResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Simplified cog source kind for resolver construction.
/// Maps from the CLI's CogSource enum without requiring the full registry dependency.
#[derive(Debug, Clone)]
pub enum CogSourceKind {
    /// From the cog registry (default)
    Registry,
    /// Local filesystem path
    Path(PathBuf),
    /// Git repository
    Git { rev: Text },
    /// IPFS content-addressed
    Ipfs { hash: Text },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_resolver() {
        let resolver = CogResolver::new();
        assert!(!resolver.is_external_cog("http"));
        assert_eq!(resolver.cog_count(), 0);
    }

    #[test]
    fn test_register_and_resolve() {
        let mut resolver = CogResolver::new();
        resolver.register_cog("http", "1.2.3", PathBuf::from("/path/to/http/src"));

        assert!(resolver.is_external_cog("http"));
        assert!(!resolver.is_external_cog("json"));
        assert_eq!(
            resolver.get_cog_root("http"),
            Some(&PathBuf::from("/path/to/http/src"))
        );
    }

    #[test]
    fn test_from_locked_cogs() {
        let locked = vec![
            (Text::from("http"), Text::from("1.2.3"), CogSourceKind::Registry),
            (Text::from("local_lib"), Text::from("0.1.0"), CogSourceKind::Path(PathBuf::from("../local_lib/src"))),
        ];
        let resolver = CogResolver::from_locked_cogs(&locked);

        assert!(resolver.is_external_cog("http"));
        assert!(resolver.is_external_cog("local_lib"));
        assert_eq!(
            resolver.get_cog_root("local_lib"),
            Some(&PathBuf::from("../local_lib/src"))
        );
    }
}
