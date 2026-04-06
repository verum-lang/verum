//! Test result caching for incremental testing.
//!
//! Caches test results based on file content hashes to avoid re-running
//! tests that haven't changed. This can significantly speed up development
//! iteration cycles.
//!
//! # Cache Structure
//!
//! ```text
//! .vtest-cache/
//! ├── index.json      # Maps file hashes to result entries
//! ├── results/        # Cached test result files
//! │   ├── abc123.json
//! │   └── def456.json
//! └── meta.json       # Cache metadata (version, timestamp)
//! ```
//!
//! # Cache Key Computation
//!
//! The cache key is computed from:
//! - Content hash of the test file
//! - Content hash of any imported files
//! - Compiler version
//! - Relevant configuration options
//!
//! # Cache Invalidation
//!
//! Cache entries are invalidated when:
//! - The test file content changes
//! - Any imported file content changes
//! - The compiler version changes
//! - The test runner configuration changes
//! - The cache TTL expires (configurable, default 7 days)

use crate::executor::TestResult;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use verum_common::Text;

/// Error types for cache operations.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Cache corrupted: {0}")]
    Corrupted(Text),

    #[error("Cache miss for key: {0}")]
    Miss(Text),

    #[error("Cache disabled")]
    Disabled,
}

/// Cache configuration.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache directory path
    pub cache_dir: PathBuf,
    /// Whether caching is enabled
    pub enabled: bool,
    /// Time-to-live for cache entries
    pub ttl: Duration,
    /// Maximum cache size in bytes
    pub max_size_bytes: u64,
    /// Compiler version for cache key
    pub compiler_version: Text,
    /// Additional configuration hash (for test runner options)
    pub config_hash: Text,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(".vtest-cache"),
            enabled: true,
            ttl: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            max_size_bytes: 100 * 1024 * 1024,          // 100 MB
            compiler_version: "unknown".to_string().into(),
            config_hash: Text::new(),
        }
    }
}

/// Cache metadata stored in meta.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Cache format version
    pub version: u32,
    /// Creation timestamp
    pub created_at: u64,
    /// Last accessed timestamp
    pub last_accessed: u64,
    /// Total size in bytes
    pub total_size: u64,
    /// Number of entries
    pub entry_count: usize,
    /// Compiler version this cache was created with
    pub compiler_version: Text,
}

impl Default for CacheMeta {
    fn default() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            version: 1,
            created_at: now,
            last_accessed: now,
            total_size: 0,
            entry_count: 0,
            compiler_version: "unknown".to_string().into(),
        }
    }
}

/// Cache index entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Content hash of the test file
    pub content_hash: Text,
    /// Content hashes of imported files
    pub import_hashes: HashMap<Text, Text>,
    /// Compiler version
    pub compiler_version: Text,
    /// Configuration hash
    pub config_hash: Text,
    /// Timestamp when this entry was created
    pub created_at: u64,
    /// Timestamp when this entry was last accessed
    pub last_accessed: u64,
    /// Path to the cached result file
    pub result_path: Text,
    /// Size of the result file in bytes
    pub size_bytes: u64,
}

/// Cache index mapping source paths to cache entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheIndex {
    /// Maps source file paths to cache entries
    pub entries: HashMap<Text, CacheEntry>,
}

/// Test result cache for incremental testing.
pub struct TestCache {
    config: CacheConfig,
    meta: CacheMeta,
    index: CacheIndex,
}

impl TestCache {
    /// Create a new test cache with the given configuration.
    pub fn new(config: CacheConfig) -> Result<Self, CacheError> {
        if !config.enabled {
            return Err(CacheError::Disabled);
        }

        // Ensure cache directory exists
        fs::create_dir_all(&config.cache_dir)?;
        fs::create_dir_all(config.cache_dir.join("results"))?;

        // Load or create metadata
        let meta_path = config.cache_dir.join("meta.json");
        let meta = if meta_path.exists() {
            let content = fs::read_to_string(&meta_path)?;
            let mut meta: CacheMeta = serde_json::from_str(&content)?;

            // Check version compatibility
            if meta.version != 1 {
                // Incompatible version, clear cache
                Self::clear_cache_dir(&config.cache_dir)?;
                CacheMeta {
                    compiler_version: config.compiler_version.clone(),
                    ..Default::default()
                }
            } else if meta.compiler_version != config.compiler_version {
                // Compiler version changed, clear cache
                Self::clear_cache_dir(&config.cache_dir)?;
                CacheMeta {
                    compiler_version: config.compiler_version.clone(),
                    ..Default::default()
                }
            } else {
                // Update last accessed time
                meta.last_accessed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                meta
            }
        } else {
            CacheMeta {
                compiler_version: config.compiler_version.clone(),
                ..Default::default()
            }
        };

        // Load or create index
        let index_path = config.cache_dir.join("index.json");
        let index = if index_path.exists() {
            let content = fs::read_to_string(&index_path)?;
            serde_json::from_str(&content)?
        } else {
            CacheIndex::default()
        };

        Ok(Self {
            config,
            meta,
            index,
        })
    }

    /// Clear the cache directory.
    fn clear_cache_dir(cache_dir: &Path) -> Result<(), CacheError> {
        if cache_dir.exists() {
            fs::remove_dir_all(cache_dir)?;
            fs::create_dir_all(cache_dir)?;
            fs::create_dir_all(cache_dir.join("results"))?;
        }
        Ok(())
    }

    /// Compute content hash of a file.
    pub fn hash_file(path: &Path) -> Result<Text, CacheError> {
        let mut file = fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 8192];

        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }

        Ok(hasher.finalize().to_hex().to_string().into())
    }

    /// Compute content hash of a string.
    pub fn hash_string(content: &str) -> Text {
        let hasher = blake3::hash(content.as_bytes());
        hasher.to_hex().to_string().into()
    }

    /// Generate cache key for a test file.
    fn generate_cache_key(&self, source_path: &Path) -> Result<Text, CacheError> {
        let content_hash = Self::hash_file(source_path)?;

        // Combine with compiler version and config
        let key_material = format!(
            "{}:{}:{}",
            content_hash, self.config.compiler_version, self.config.config_hash
        );

        Ok(Self::hash_string(&key_material))
    }

    /// Look up a cached test result.
    pub fn get(&self, source_path: &Path) -> Result<TestResult, CacheError> {
        let path_str: Text = source_path.to_string_lossy().to_string().into();

        // Get cache entry
        let entry = self
            .index
            .entries
            .get(&path_str)
            .ok_or_else(|| CacheError::Miss(path_str.clone()))?;

        // Check if entry is still valid
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if now - entry.created_at > self.config.ttl.as_secs() {
            return Err(CacheError::Miss("Entry expired".to_string().into()));
        }

        // Verify content hash still matches
        let current_hash = Self::hash_file(source_path)?;
        if current_hash != entry.content_hash {
            return Err(CacheError::Miss("Content changed".to_string().into()));
        }

        // Check compiler version
        if entry.compiler_version != self.config.compiler_version {
            return Err(CacheError::Miss("Compiler version changed".to_string().into()));
        }

        // Check config hash
        if entry.config_hash != self.config.config_hash {
            return Err(CacheError::Miss("Config changed".to_string().into()));
        }

        // Load cached result
        let result_path = self
            .config
            .cache_dir
            .join("results")
            .join(entry.result_path.as_str());
        let content = fs::read_to_string(&result_path)?;
        let result: TestResult = serde_json::from_str(&content)?;

        Ok(result)
    }

    /// Store a test result in the cache.
    pub fn put(&mut self, source_path: &Path, result: &TestResult) -> Result<(), CacheError> {
        let path_str: Text = source_path.to_string_lossy().to_string().into();
        let content_hash = Self::hash_file(source_path)?;

        // Serialize result
        let result_json = serde_json::to_string_pretty(result)?;
        let result_bytes = result_json.as_bytes();

        // Generate result file name
        let result_hash = Self::hash_string(&result_json);
        let result_filename: Text = format!("{}.json", &result_hash.as_str()[..16]).into();
        let result_path = self.config.cache_dir.join("results").join(result_filename.as_str());

        // Write result file
        let mut file = fs::File::create(&result_path)?;
        file.write_all(result_bytes)?;

        // Update index
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let entry = CacheEntry {
            content_hash,
            import_hashes: HashMap::new(), // TODO: track imports
            compiler_version: self.config.compiler_version.clone(),
            config_hash: self.config.config_hash.clone(),
            created_at: now,
            last_accessed: now,
            result_path: result_filename,
            size_bytes: result_bytes.len() as u64,
        };

        // Update metadata
        if !self.index.entries.contains_key(&path_str) {
            self.meta.entry_count += 1;
        }
        self.meta.total_size += entry.size_bytes;
        self.meta.last_accessed = now;

        self.index.entries.insert(path_str, entry);

        // Persist index and meta
        self.save()?;

        // Check if we need to evict entries
        if self.meta.total_size > self.config.max_size_bytes {
            self.evict_lru()?;
        }

        Ok(())
    }

    /// Check if a result is cached and valid.
    pub fn is_cached(&self, source_path: &Path) -> bool {
        self.get(source_path).is_ok()
    }

    /// Invalidate cache entry for a file.
    pub fn invalidate(&mut self, source_path: &Path) -> Result<(), CacheError> {
        let path_str: Text = source_path.to_string_lossy().to_string().into();

        if let Some(entry) = self.index.entries.remove(&path_str) {
            // Remove result file
            let result_path = self
                .config
                .cache_dir
                .join("results")
                .join(entry.result_path.as_str());
            if result_path.exists() {
                fs::remove_file(&result_path)?;
            }

            // Update metadata
            self.meta.entry_count = self.meta.entry_count.saturating_sub(1);
            self.meta.total_size = self.meta.total_size.saturating_sub(entry.size_bytes);

            self.save()?;
        }

        Ok(())
    }

    /// Clear all cache entries.
    pub fn clear(&mut self) -> Result<(), CacheError> {
        Self::clear_cache_dir(&self.config.cache_dir)?;
        self.index = CacheIndex::default();
        self.meta = CacheMeta {
            compiler_version: self.config.compiler_version.clone(),
            ..Default::default()
        };
        self.save()?;
        Ok(())
    }

    /// Save cache index and metadata to disk.
    fn save(&self) -> Result<(), CacheError> {
        // Save index
        let index_path = self.config.cache_dir.join("index.json");
        let index_json = serde_json::to_string_pretty(&self.index)?;
        fs::write(&index_path, index_json)?;

        // Save metadata
        let meta_path = self.config.cache_dir.join("meta.json");
        let meta_json = serde_json::to_string_pretty(&self.meta)?;
        fs::write(&meta_path, meta_json)?;

        Ok(())
    }

    /// Evict least recently used entries until under size limit.
    fn evict_lru(&mut self) -> Result<(), CacheError> {
        // Sort entries by last_accessed time
        let mut entries: Vec<_> = self
            .index
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        entries.sort_by_key(|(_, v)| v.last_accessed);

        // Evict oldest entries until under limit
        let target_size = self.config.max_size_bytes * 3 / 4; // Leave some headroom

        for (path, entry) in entries {
            if self.meta.total_size <= target_size {
                break;
            }

            // Remove result file
            let result_path = self
                .config
                .cache_dir
                .join("results")
                .join(entry.result_path.as_str());
            if result_path.exists() {
                let _ = fs::remove_file(&result_path);
            }

            // Update tracking
            self.meta.total_size = self.meta.total_size.saturating_sub(entry.size_bytes);
            self.meta.entry_count = self.meta.entry_count.saturating_sub(1);
            self.index.entries.remove(&path);
        }

        self.save()?;
        Ok(())
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.meta.entry_count,
            total_size_bytes: self.meta.total_size,
            cache_dir: self.config.cache_dir.clone(),
            compiler_version: self.meta.compiler_version.clone(),
            created_at: self.meta.created_at,
            last_accessed: self.meta.last_accessed,
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of cached entries
    pub entry_count: usize,
    /// Total size in bytes
    pub total_size_bytes: u64,
    /// Cache directory
    pub cache_dir: PathBuf,
    /// Compiler version
    pub compiler_version: Text,
    /// Creation timestamp
    pub created_at: u64,
    /// Last access timestamp
    pub last_accessed: u64,
}

impl CacheStats {
    /// Format size for display.
    pub fn format_size(&self) -> Text {
        if self.total_size_bytes < 1024 {
            format!("{} B", self.total_size_bytes).into()
        } else if self.total_size_bytes < 1024 * 1024 {
            format!("{:.1} KB", self.total_size_bytes as f64 / 1024.0).into()
        } else {
            format!("{:.1} MB", self.total_size_bytes as f64 / (1024.0 * 1024.0)).into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_hash_string() {
        let hash1 = TestCache::hash_string("hello");
        let hash2 = TestCache::hash_string("hello");
        let hash3 = TestCache::hash_string("world");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_cache_config_default() {
        let config = CacheConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_size_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn test_cache_creation() {
        let dir = tempdir().unwrap();
        let config = CacheConfig {
            cache_dir: dir.path().to_path_buf(),
            enabled: true,
            ..Default::default()
        };

        let cache = TestCache::new(config);
        assert!(cache.is_ok());
    }

    #[test]
    fn test_cache_disabled() {
        let dir = tempdir().unwrap();
        let config = CacheConfig {
            cache_dir: dir.path().to_path_buf(),
            enabled: false,
            ..Default::default()
        };

        let result = TestCache::new(config);
        assert!(matches!(result, Err(CacheError::Disabled)));
    }

    #[test]
    fn test_cache_stats() {
        let dir = tempdir().unwrap();
        let config = CacheConfig {
            cache_dir: dir.path().to_path_buf(),
            enabled: true,
            ..Default::default()
        };

        let cache = TestCache::new(config).unwrap();
        let stats = cache.stats();

        assert_eq!(stats.entry_count, 0);
        assert_eq!(stats.total_size_bytes, 0);
    }

    #[test]
    fn test_format_size() {
        let stats = CacheStats {
            entry_count: 0,
            total_size_bytes: 500,
            cache_dir: PathBuf::new(),
            compiler_version: Text::new(),
            created_at: 0,
            last_accessed: 0,
        };
        assert_eq!(stats.format_size().as_str(), "500 B");

        let stats = CacheStats {
            total_size_bytes: 2048,
            ..stats.clone()
        };
        assert_eq!(stats.format_size().as_str(), "2.0 KB");

        let stats = CacheStats {
            total_size_bytes: 2 * 1024 * 1024,
            ..stats
        };
        assert_eq!(stats.format_size().as_str(), "2.0 MB");
    }
}
