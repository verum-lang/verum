// Cog lockfile: deterministic dependency resolution, integrity hashes, reproducible builds

use super::types::CogSource;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use verum_common::{List, Map, Text};

/// Verum lockfile (verum.lock)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    /// Lockfile format version
    pub version: u32,

    /// Root package name
    pub root: Text,

    /// Locked dependencies
    pub packages: List<LockedCog>,

    /// Metadata
    pub metadata: LockfileMetadata,
}

/// Locked cog with exact version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedCog {
    /// Cog name
    pub name: Text,

    /// Exact version
    pub version: Text,

    /// Cog source
    pub source: CogSource,

    /// SHA-256 checksum
    pub checksum: Text,

    /// Dependencies (name -> version)
    pub dependencies: Map<Text, Text>,

    /// Enabled features
    pub features: List<Text>,

    /// Optional dependency
    #[serde(default)]
    pub optional: bool,
}

/// Lockfile metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LockfileMetadata {
    /// Creation timestamp
    pub created_at: i64,

    /// Last updated timestamp
    pub updated_at: i64,

    /// Verum CLI version that created this
    pub cli_version: Text,
}

impl Lockfile {
    /// Create new lockfile
    pub fn new(root: Text) -> Self {
        Self {
            version: 1,
            root,
            packages: List::new(),
            metadata: LockfileMetadata {
                created_at: chrono::Utc::now().timestamp(),
                updated_at: chrono::Utc::now().timestamp(),
                cli_version: env!("CARGO_PKG_VERSION").into(),
            },
        }
    }

    /// Load lockfile from disk
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let lockfile: Lockfile = toml::from_str(&content)?;
        Ok(lockfile)
    }

    /// Save lockfile to disk
    pub fn to_file(&self, path: &Path) -> Result<()> {
        let mut updated = self.clone();
        updated.metadata.updated_at = chrono::Utc::now().timestamp();

        let content = toml::to_string_pretty(&updated)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add locked package
    pub fn add_cog(&mut self, package: LockedCog) {
        // Remove existing entry if present
        self.packages.retain(|p| p.name != package.name);
        self.packages.push(package);
        self.sort_cogs();
    }

    /// Remove locked package
    pub fn remove_cog(&mut self, name: &str) -> bool {
        let original_len = self.packages.len();
        self.packages.retain(|p| p.name != name);
        self.packages.len() != original_len
    }

    /// Get locked package
    pub fn get_cog(&self, name: &str) -> Option<&LockedCog> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Update package version
    pub fn update_cog(&mut self, name: &str, version: Text, checksum: Text) -> bool {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            pkg.version = version;
            pkg.checksum = checksum;
            true
        } else {
            false
        }
    }

    /// Sort packages alphabetically
    fn sort_cogs(&mut self) {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Verify checksums
    pub fn verify_checksums(&self, cache_dir: &Path) -> Result<List<Text>> {
        let mut failed = List::new();

        for package in &self.packages {
            let cog_path = cache_dir
                .join(&package.name)
                .join(&package.version)
                .join(format!("{}-{}.tar.gz", package.name, package.version));

            if cog_path.exists() {
                let actual_checksum = calculate_checksum(&cog_path)?;
                if actual_checksum != package.checksum {
                    failed.push(
                        format!("{} {}: checksum mismatch", package.name, package.version).into(),
                    );
                }
            }
        }

        Ok(failed)
    }

    /// Check if lockfile is up to date with manifest
    pub fn is_up_to_date(&self, manifest_dependencies: &Map<Text, Text>) -> bool {
        let locked_deps: Map<_, _> = self
            .packages
            .iter()
            .map(|p| (p.name.as_str(), p.version.as_str()))
            .collect();

        for name in manifest_dependencies.keys() {
            if !locked_deps.contains_key(&name.as_str()) {
                return false;
            }
        }

        true
    }

    /// Get dependency graph
    pub fn dependency_graph(&self) -> Map<Text, List<Text>> {
        self.packages
            .iter()
            .map(|p| (p.name.clone(), p.dependencies.keys().cloned().collect()))
            .collect()
    }
}

/// Calculate SHA-256 checksum of file
fn calculate_checksum(path: &Path) -> Result<Text> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(format!("{:x}", hasher.finalize()).into())
}

/// Create lockfile from resolved dependencies
pub fn create_from_resolved(
    root: Text,
    resolved: List<super::resolver::ResolvedDependency>,
    checksums: Map<Text, Text>,
) -> Lockfile {
    let mut lockfile = Lockfile::new(root);

    for dep in &resolved {
        let checksum = checksums.get(&dep.name).cloned().unwrap_or_else(Text::new);

        let locked = LockedCog {
            name: dep.name.clone(),
            version: dep.version.to_string().into(),
            source: dep.source.clone(),
            checksum,
            dependencies: dep
                .dependencies
                .iter()
                .map(|d| {
                    let version = resolved
                        .iter()
                        .find(|r| &r.name == d)
                        .map(|r| r.version.to_string().into())
                        .unwrap_or_default();
                    (d.clone(), version)
                })
                .collect(),
            features: dep.features.iter().cloned().collect(),
            optional: false,
        };

        lockfile.add_cog(locked);
    }

    lockfile
}

// Add chrono for timestamps
use chrono;
