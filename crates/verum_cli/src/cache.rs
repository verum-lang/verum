// Build cache for incremental compilation
// Implements SHA-256 based file change detection and artifact caching

use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use verum_common::{List, Map, Text};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCache {
    pub version: u32,
    pub files: Map<PathBuf, FileEntry>,
    pub artifacts: Map<PathBuf, ArtifactEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub hash: Text,
    pub timestamp: SystemTime,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactEntry {
    pub source_files: List<PathBuf>,
    pub dependencies: List<Text>,
    pub output: PathBuf,
    pub timestamp: SystemTime,
}

pub const CACHE_VERSION: u32 = 1;
const CACHE_FILE: &str = ".verum_cache";

impl BuildCache {
    pub fn new() -> Self {
        Self {
            version: CACHE_VERSION,
            files: Map::new(),
            artifacts: Map::new(),
        }
    }

    pub fn load(project_dir: &Path) -> Result<Self> {
        let cache_path = project_dir.join(CACHE_FILE);

        if !cache_path.exists() {
            return Ok(Self::new());
        }

        let content = std::fs::read_to_string(&cache_path)?;
        let cache: BuildCache =
            serde_json::from_str(&content).map_err(|_| CliError::CacheCorrupted)?;

        if cache.version != CACHE_VERSION {
            // Version mismatch, start fresh
            Ok(Self::new())
        } else {
            Ok(cache)
        }
    }

    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let cache_path = project_dir.join(CACHE_FILE);
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CliError::Custom(format!("Failed to serialize cache: {}", e)))?;

        std::fs::write(cache_path, content)?;
        Ok(())
    }

    pub fn is_file_changed(&self, path: &Path) -> Result<bool> {
        let current_hash = compute_file_hash(path)?;

        match self.files.get(&path.to_path_buf()) {
            Some(entry) => Ok(entry.hash != current_hash),
            None => Ok(true), // File not in cache, treat as changed
        }
    }

    pub fn update_file(&mut self, path: &Path) -> Result<()> {
        let hash = compute_file_hash(path)?;
        let metadata = std::fs::metadata(path)?;

        self.files.insert(
            path.to_path_buf(),
            FileEntry {
                hash,
                timestamp: metadata.modified()?,
                size: metadata.len(),
            },
        );

        Ok(())
    }

    pub fn get_changed_files(&self, files: &[PathBuf]) -> Result<List<PathBuf>> {
        let mut changed = List::new();

        for file in files {
            if self.is_file_changed(file)? {
                changed.push(file.clone());
            }
        }

        Ok(changed)
    }

    pub fn add_artifact(
        &mut self,
        source_files: List<PathBuf>,
        dependencies: List<Text>,
        output: PathBuf,
    ) {
        let key = output.clone();
        self.artifacts.insert(
            key,
            ArtifactEntry {
                source_files,
                dependencies,
                output,
                timestamp: SystemTime::now(),
            },
        );
    }

    pub fn is_artifact_valid(&self, artifact: &Path) -> Result<bool> {
        let entry = match self.artifacts.get(&artifact.to_path_buf()) {
            Some(entry) => entry,
            None => return Ok(false),
        };

        // Check if artifact file exists
        if !entry.output.exists() {
            return Ok(false);
        }

        // Check if any source files have changed
        for source in &entry.source_files {
            if self.is_file_changed(source)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub fn invalidate_artifact(&mut self, artifact: &Path) {
        self.artifacts.remove(&artifact.to_path_buf());
    }

    pub fn clear(&mut self) {
        self.files.clear();
        self.artifacts.clear();
    }

    pub fn prune_missing(&mut self) {
        self.files.retain(|path, _| path.exists());
        self.artifacts.retain(|_, entry| entry.output.exists());
    }

    pub fn get_stats(&self) -> CacheStats {
        let total_size: u64 = self.files.values().map(|e| e.size).sum();

        CacheStats {
            file_count: self.files.len(),
            artifact_count: self.artifacts.len(),
            total_size,
        }
    }
}

#[derive(Debug)]
pub struct CacheStats {
    pub file_count: usize,
    pub artifact_count: usize,
    pub total_size: u64,
}

pub fn compute_file_hash(path: &Path) -> Result<Text> {
    let content = std::fs::read(path)?;
    let hash = blake3::hash(&content);
    Ok(hash.to_hex().to_string().into())
}

pub fn compute_content_hash(content: &[u8]) -> Text {
    let hash = blake3::hash(content);
    hash.to_hex().to_string().into()
}

impl Default for BuildCache {
    fn default() -> Self {
        Self::new()
    }
}
